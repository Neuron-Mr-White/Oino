//! `unipi.mcp` — MCP client and dynamic tools (unipi `packages/mcp`, unipi-port §3.22).
//!
//! | unipi | here |
//! |---|---|
//! | stdio JSON-RPC client, 10 s timeout, initialize handshake | [`client`] over host `process` pipes; plus streamable HTTP over `http.fetch` (pi `mcp.json` `url` entries) |
//! | config merge global/project | canonical global + four trusted project files, migration, scoped writes and secrets ([`storage`]); ACP servers ([`config`]) |
//! | parallel connect, registration barrier, sorted `<server>__<tool>`, canonical schemas, ≤ 20 servers | [`registry`], [`translate`] |
//! | 64 KiB head/tail bound + 0600 spill | `unipi_kit::bounded` |
//! | `/mcp-status`, runners `mcp-add`/`-settings`/`-sync`/`-reload` | reload is visible and works live |
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
mod storage;
mod translate;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::rc::Rc;

use futures::future::join_all;
use oino_extension::prelude::*;
use oino_extension::sync::SessionMap;
use oino_extension::ui::KeyBinding;
use oino_extension::ui::widgets::prelude::*;
use oino_extension::wit::registry::{Exposure, ToolAnnotations};
use unipi_kit::bus::Delivery;
use unipi_kit::settings::{Field, Namespace, Section};
use unipi_kit::topics::{self, McpServer, McpTools, ModuleReady};

use crate::client::{Client, Target};
use crate::config::{Resolved, ServerDef, Source, Transport};
use crate::registry::{Registry, Status};
use crate::translate::ToolDef;

/// Extension id.
const EXTENSION_ID: &str = "unipi.mcp";
const VERSION: &str = "3.0.0";
/// Canonical global file through a profile-re-rooted writable grant.
const PI_CONFIG: &str = "/mcp-agent/mcp.json";
const CATALOG_CACHE: &str = "/ext-global/servers.json";
const PROJECT_CONFIG: &str = "/project/.oino/mcp.json";

#[derive(Clone, Debug)]
enum View {
    Servers,
    Server(String),
    Catalog(String),
}

/// A row of action buttons.
fn buttons(items: &[(&str, &str, Option<ButtonVariant>)]) -> Node {
    let children = items.iter().map(|(label, action, variant)| {
        let button = Button::new(*label, *action);
        let button = match variant {
            Some(v) => button.variant(*v),
            None => button,
        };
        button.into()
    });
    Layout::horizontal(vec![Constraint::Min(0); items.len()])
        .spacing(1)
        .children(children)
        .into()
}

/// A section header: a top rule with the title (`section` in the §12.1 mapping).
fn section(title: &str, body: impl Into<Node>) -> Node {
    Block::default()
        .borders(Borders::Sides(vec![BorderSide::TOP]))
        .border_style(Style::new().fg(Color::BorderMuted))
        .title(title)
        .child(body.into())
        .into()
}

/// Key/value rows (`kv` → 2-column `Table`, §12.1).
fn kv(items: &[(&str, String)]) -> Node {
    let rows = items.iter().map(|(key, value)| {
        let key_cell = oino_extension::ui::widgets::widgets::table::Cell::new(Line::from(
            Span::from_string(*key).fg(Color::Muted),
        ));
        Row::new([
            key_cell,
            oino_extension::ui::widgets::widgets::table::Cell::new(value.clone()),
        ])
    });
    Table::new(rows, [Constraint::Max(24), Constraint::Fill(1)]).into()
}

/// Status tone token.
fn status_color(status: Status) -> Color {
    match status {
        Status::Running => Color::Success,
        Status::Error => Color::Error,
        Status::Starting => Color::Warning,
        Status::Stopped => Color::Muted,
    }
}

/// The extension.
#[derive(Default)]
pub struct Mcp {
    registry: RefCell<Registry>,
    clients: RefCell<BTreeMap<String, Rc<Client>>>,
    acp: RefCell<Vec<(String, ServerDef)>>,
    errors: RefCell<Vec<String>>,
    fallbacks: RefCell<BTreeMap<String, Resolved>>,
    fallback_warnings: RefCell<BTreeMap<String, String>>,
    active_sessions: Cell<usize>,
    started: Cell<bool>,
    /// Per-server tool set last announced as available (not yet withdrawn).
    announced_tools: RefCell<BTreeMap<String, BTreeSet<String>>>,
    panels: SessionMap<BTreeMap<String, View>>,
}

fn now_iso() -> String {
    unipi_kit::fmt::iso_from_ms(timers::now_ms())
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

    fn project_secret_prefix() -> String {
        let path = oino_extension::fs::preopens()
            .into_iter()
            .find(|p| p.guest_path.trim_end_matches('/') == "/project")
            .map(|p| p.host_path)
            .unwrap_or_default();
        storage::project_secret_prefix(&path)
    }

    async fn load(&self) -> (Vec<Resolved>, Vec<String>) {
        let Ok(_lock) = oino_extension::fs::Lock::acquire("/mcp-agent/mcp.lock", true).await else {
            return (
                Vec::new(),
                vec!["Global MCP configuration: unreadable (lock)".into()],
            );
        };
        let migration = storage::migrate(
            std::path::Path::new(PI_CONFIG),
            std::path::Path::new("/ext-global"),
            &|p, v| write_private(&p.to_string_lossy(), v).map_err(|e| e.to_string()),
        );
        // Migrate only Oino's own trusted project file. Compatibility files stay read-only.
        let mut migration_error = None;
        if control::is_project_trusted() && std::path::Path::new(PROJECT_CONFIG).exists() {
            let result = storage::edit(std::path::Path::new(PROJECT_CONFIG), |entries| {
                for entry in entries.values_mut() {
                    storage::scope_secrets(
                        entry,
                        &Self::project_secret_prefix(),
                        &Self::lookup_var,
                        &|key, value| {
                            secrets::set(key, value)
                                .map_err(|_| "Could not migrate MCP secret".into())
                        },
                    )?;
                }
                Ok(())
            });
            match result {
                Ok(doc) => {
                    let old = std::fs::read_to_string(PROJECT_CONFIG)
                        .ok()
                        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
                    if old.as_ref() != Some(&doc)
                        && let Err(e) = write_private(PROJECT_CONFIG, &doc)
                    {
                        migration_error = Some(e.to_string());
                    }
                }
                Err(e) => migration_error = Some(e),
            }
        }
        let mut dedupe_error = Self::dedupe_secret_names_in(PI_CONFIG, "");
        if control::is_project_trusted() {
            dedupe_error =
                Self::dedupe_secret_names_in(PROJECT_CONFIG, &Self::project_secret_prefix())
                    .or(dedupe_error);
        }
        let (resolved, mut errors) = storage::load(
            std::path::Path::new(PI_CONFIG),
            std::path::Path::new("/project"),
            control::is_project_trusted(),
            &self.acp.borrow(),
        );
        let (global, _) = storage::load(
            std::path::Path::new(PI_CONFIG),
            std::path::Path::new("/project"),
            false,
            &[],
        );
        *self.fallbacks.borrow_mut() = global
            .into_iter()
            .filter(|r| r.enabled)
            .map(|r| (r.name.clone(), r))
            .collect();
        if let Some(e) = migration_error {
            errors.push(e);
        }
        if let Some(e) = dedupe_error {
            errors.push(e);
        }
        if let Err(e) = migration {
            errors.push(e);
        }
        (resolved, errors)
    }

    fn lookup_var(name: &str) -> Option<String> {
        secrets::get(name).ok().flatten()
    }

    /// N8: rewrite `${…REFERO_REFERO_AUTH}` references to `${…REFERO_AUTH}` in one Oino-owned file (the value is copied,
    /// never lost). Parse errors are left to `storage::load`, which reports them.
    fn dedupe_secret_names_in(path: &str, prefix: &str) -> Option<String> {
        if !std::path::Path::new(path).exists() {
            return None;
        }
        let doc = storage::edit(std::path::Path::new(path), |entries| {
            for (name, entry) in entries.iter_mut() {
                storage::dedupe_secret_names(
                    name,
                    entry,
                    prefix,
                    &Self::lookup_var,
                    &|key, value| {
                        secrets::set(key, value).map_err(|_| "Could not migrate MCP secret".into())
                    },
                )?;
            }
            Ok(())
        })
        .ok()?;
        let old = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok());
        if old.as_ref() == Some(&doc) {
            return None;
        }
        write_private(path, &doc).err().map(|e| e.to_string())
    }

    async fn run_value_command(cmd: &str) -> Option<String> {
        let windows = host::info().platform == "windows";
        let (prog, flag) = if windows { ("cmd", "/C") } else { ("sh", "-c") };
        let r = process::exec(prog, &[flag, cmd], None, Some(10_000))
            .await
            .ok()?;
        (r.code == Some(0)).then(|| r.stdout.trim().to_owned())
    }

    async fn expand_value(
        value: &str,
        env: &BTreeMap<String, String>,
    ) -> std::result::Result<String, String> {
        if let Some(cmd) = value.strip_prefix('!') {
            let value = Self::run_value_command(cmd)
                .await
                .ok_or_else(|| "MCP value command failed".to_owned())?;
            return storage::expand(&value, &|key| {
                env.get(key).cloned().or_else(|| Self::lookup_var(key))
            });
        }
        storage::expand(value, &|key| {
            env.get(key).cloned().or_else(|| Self::lookup_var(key))
        })
    }

    /// Resolve everything before opening a transport. Missing secrets never reach a server.
    async fn target(def: &ServerDef) -> std::result::Result<Target, String> {
        let home = host::env("HOME")
            .ok()
            .flatten()
            .or_else(|| host::env("USERPROFILE").ok().flatten());
        // Resolve env values against secrets first (self references such as
        // TOKEN=${TOKEN} must not recurse); args/headers/url then see this env.
        let mut resolved_env = BTreeMap::new();
        for (key, value) in &def.env {
            resolved_env.insert(
                key.clone(),
                Self::expand_value(value, &BTreeMap::new()).await?,
            );
        }
        Ok(match def.transport {
            Transport::Stdio => {
                let mut env = Vec::new();
                for (k, v) in &def.env {
                    env.push((k.clone(), Self::expand_value(v, &resolved_env).await?));
                }
                let mut args = Vec::new();
                for a in &def.args {
                    args.push(config::expand_home(
                        &Self::expand_value(a, &resolved_env).await?,
                        home.as_deref(),
                    ));
                }
                Target::Stdio {
                    command: config::expand_home(
                        &Self::expand_value(&def.command, &resolved_env).await?,
                        home.as_deref(),
                    ),
                    args,
                    env,
                    cwd: match &def.cwd {
                        Some(c) => Some(config::expand_home(
                            &Self::expand_value(c, &resolved_env).await?,
                            home.as_deref(),
                        )),
                        None => None,
                    },
                }
            }
            Transport::Http => {
                let mut headers = Vec::new();
                for (k, v) in &def.headers {
                    headers.push((k.clone(), Self::expand_value(v, &resolved_env).await?));
                }
                Target::Http {
                    url: Self::expand_value(&def.url, &resolved_env).await?,
                    headers,
                }
            }
        })
    }

    // ── lifecycle ───────────────────────────────────────────────────────────

    async fn connect(r: &Resolved) -> Result<(Client, Vec<ToolDef>), String> {
        let timeout = r.def.timeout_ms.unwrap_or(config::STARTUP_TIMEOUT_MS);
        let target = Self::target(&r.def).await?;
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

    /// Records an available tool set and returns the reminder for its delta.
    /// A restart that brings the same set announces nothing.
    fn announced_available(&self, server: &str, names: &[String]) -> Option<String> {
        let current: BTreeSet<String> = names.iter().cloned().collect();
        let mut announced = self.announced_tools.borrow_mut();
        let previous = announced.get(server).cloned().unwrap_or_default();
        if previous == current {
            return None;
        }
        announced.insert(server.to_owned(), current.clone());
        let added: Vec<String> = current.difference(&previous).cloned().collect();
        let removed: Vec<String> = previous.difference(&current).cloned().collect();
        if added.is_empty() && removed.is_empty() {
            return None;
        }
        let mut message = String::from("MCP catalog update.");
        if !added.is_empty() {
            let _ = write!(
                message,
                " Newly available deferred tools: {}. Discover tools before calling them.",
                added.join(", ")
            );
        }
        if !removed.is_empty() {
            let _ = write!(message, " Unavailable tools: {}.", removed.join(", "));
        }
        Some(message)
    }

    /// Withdraws a previously announced set; `None` when nothing is outstanding.
    fn announced_unavailable(&self, server: &str) -> Option<String> {
        let set = self.announced_tools.borrow_mut().remove(server)?;
        if set.is_empty() {
            return None;
        }
        let names: Vec<String> = set.into_iter().collect();
        Some(format!(
            "MCP catalog update. Unavailable tools: {}.",
            names.join(", ")
        ))
    }

    fn announce_tools(&self, server: &str, names: &[String]) {
        if let Some(text) = self.announced_available(server, names)
            && let Err(error) = session::append_reminder("unipi.mcp/catalog", &text)
        {
            host::info_log(&format!("MCP catalog reminder failed: {error}"));
        }
    }

    fn withdraw_announced(&self, server: &str) {
        if let Some(text) = self.announced_unavailable(server)
            && let Err(error) = session::append_reminder("unipi.mcp/catalog", &text)
        {
            host::info_log(&format!("MCP catalog reminder failed: {error}"));
        }
    }

    /// unipi `startServers`: parallel connect + discovery, then one sorted registration.
    #[allow(
        clippy::too_many_lines,
        reason = "transactional registration and catalog announcement"
    )]
    async fn start_servers(
        &'static self,
        servers: Vec<Resolved>,
    ) -> std::result::Result<(), String> {
        if servers.is_empty() {
            return Ok(());
        }
        self.registry.borrow().check_batch(&servers)?;
        for s in &servers {
            self.stop_server(&s.name, false).await;
            self.registry.borrow_mut().begin(s, &now_iso());
        }
        for s in &servers {
            self.fallback_warnings.borrow_mut().remove(&s.name);
        }
        let results = join_all(servers.iter().map(Self::connect)).await;
        let mut prepared: Vec<(Resolved, Client, Vec<ToolDef>)> = Vec::new();
        for (s, r) in servers.into_iter().zip(results) {
            match r {
                Ok((c, t)) => prepared.push((s, c, t)),
                Err(e) => {
                    let fallback = self
                        .fallbacks
                        .borrow()
                        .get(&s.name)
                        .cloned()
                        .filter(|g| s.source == Source::ProjectOverride && s.def != g.def);
                    if let Some(global) = fallback
                        && let Ok((client, tools)) = Self::connect(&global).await
                    {
                        self.fallback_warnings.borrow_mut().insert(s.name.clone(), format!("Project server could not start: {e}. Using global server; Reconnect retries the project entry."));
                        self.registry.borrow_mut().begin(&global, &now_iso());
                        prepared.push((global, client, tools));
                        continue;
                    }
                    self.registry.borrow_mut().fail(&s.name, &e);
                    self.withdraw_announced(&s.name);
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
        let mut by_server: BTreeMap<String, Vec<String>> = BTreeMap::new();
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
            by_server
                .entry(server.clone())
                .or_default()
                .push(def.name.clone());
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
            } else {
                // A restart that now exposes no tools withdraws the old set.
                self.withdraw_announced(&s.name);
            }
        }
        if !by_server.is_empty() {
            for (server, names) in &by_server {
                self.announce_tools(server, names);
            }
        }
        Ok(())
    }

    /// unipi `stopServer`: unregister tools, disconnect.
    async fn stop_server(&self, name: &str, announce: bool) {
        let names = self.registry.borrow_mut().stop(name);
        Self::unregister(&names);
        if announce && !names.is_empty() {
            self.withdraw_announced(name);
        }
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
            self.stop_server(&n, false).await;
        }
    }

    /// Loads configs and starts every enabled server (unipi `session_start`).
    async fn start_all(&'static self) {
        let (resolved, errors) = self.load().await;
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
        self.started.set(true);
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

    /// `cwd` selects host services, never an arbitrary guest path. WI-1 mounts that
    /// project's folder at /project; do not concatenate the caller's host cwd.
    fn scope_path(p: &Value) -> std::result::Result<std::path::PathBuf, String> {
        storage::writable(
            std::path::Path::new(PI_CONFIG),
            std::path::Path::new("/project"),
            p.get("scope").and_then(Value::as_str).unwrap_or("global"),
            control::is_project_trusted(),
        )
    }

    async fn mutate(p: &Value, action: &str) -> std::result::Result<Value, String> {
        let path = Self::scope_path(p)?;
        let _lock = oino_extension::fs::Lock::acquire("/mcp-agent/mcp.lock", true)
            .await
            .map_err(|_| "Could not lock MCP configuration".to_owned())?;
        let mut moved = 0;
        // WI-3 uses a flat {VARIABLE: value} map; also accept nested
        // {server: {headerOrEnvKey: value}} for direct service clients.
        let supplied = p.get("secrets").and_then(Value::as_object);

        let doc = storage::edit(&path, |map| {
            if action == "save" {
                let entries = p
                    .get("entries")
                    .and_then(Value::as_object)
                    .ok_or("entries must be a name-to-entry object")?;
                // Validate the entire batch before writing secrets or files.
                let parsed = config::parse_file(
                    &json!({"mcpServers": entries}),
                    path != std::path::Path::new(PI_CONFIG),
                );
                if !parsed.errors.is_empty() {
                    return Err(parsed.errors.join("\n"));
                }
                moved += Self::store_supplied_secrets(supplied, entries, &path)?;
                for (name, entry) in entries {
                    if !config::valid_name(name) {
                        return Err("Invalid server name".into());
                    }
                    let mut entry = entry.clone();
                    Self::restore_redacted_entry(name, &mut entry, &path);
                    moved += storage::protect(
                        name,
                        &mut entry,
                        p.get("secrets").and_then(|s| s.get(name)),
                        path != std::path::Path::new(PI_CONFIG),
                        &|key, value| {
                            let key = if path == std::path::Path::new(PI_CONFIG) {
                                key.to_owned()
                            } else {
                                format!("{}{key}", Self::project_secret_prefix())
                            };
                            secrets::set(&key, value)
                                .map_err(|_| "Could not store MCP secret".to_owned())
                        },
                    )?;
                    if path != std::path::Path::new(PI_CONFIG) {
                        let prefix = Self::project_secret_prefix();
                        storage::scope_secrets(
                            &mut entry,
                            &prefix,
                            &|key| {
                                supplied
                                    .and_then(|s| s.get(key))
                                    .and_then(Value::as_str)
                                    .map(ToOwned::to_owned)
                                    .or_else(|| Self::lookup_var(&format!("{prefix}{key}")))
                                    .or_else(|| Self::lookup_var(key))
                            },
                            &|key, value| {
                                secrets::set(key, value)
                                    .map_err(|_| "Could not store MCP secret".to_owned())
                            },
                        )?;
                    }
                    map.insert(name.clone(), entry);
                }
            } else {
                let name = p
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|n| config::valid_name(n))
                    .ok_or("name is required")?;
                if action == "remove" {
                    map.remove(name);
                } else {
                    let enabled = p
                        .get("enabled")
                        .and_then(Value::as_bool)
                        .ok_or("enabled must be boolean")?;
                    if path == std::path::Path::new(PI_CONFIG) && !map.contains_key(name) {
                        return Err("Global server not found".into());
                    }
                    let entry = map.entry(name.to_owned()).or_insert_with(|| json!({}));
                    entry["enabled"] = json!(enabled);
                }
            }
            Ok(())
        })?;
        write_private(&path.to_string_lossy(), &doc)
            .map_err(|_| "Could not write MCP configuration".to_owned())?;
        Ok(
            json!({"ok": true, "secretsMoved": moved, "notice": if moved > 0 { Some("Secrets moved to the host secrets store") } else { None }}),
        )
    }

    fn store_supplied_secrets(
        supplied: Option<&serde_json::Map<String, Value>>,
        entries: &serde_json::Map<String, Value>,
        path: &std::path::Path,
    ) -> std::result::Result<usize, String> {
        let mut moved = 0;
        if let Some(supplied) = supplied {
            for (key, value) in supplied {
                if let Some(value) = value.as_str() {
                    let reference = format!("${{{key}}}");
                    let valid = entries.iter().any(|(server, entry)| {
                        ["env", "headers"].iter().any(|field| {
                            entry
                                .get(field)
                                .and_then(Value::as_object)
                                .is_some_and(|map| {
                                    map.iter().any(|(k, v)| {
                                        storage::secret_key(server, k) == *key
                                            && v.as_str() == Some(&reference)
                                    })
                                })
                        })
                    });
                    if !valid {
                        return Err("Secret must reference a server header or env variable".into());
                    }
                    let store_key = if path == std::path::Path::new(PI_CONFIG) {
                        key.clone()
                    } else {
                        format!("{}{key}", Self::project_secret_prefix())
                    };
                    secrets::set(&store_key, value)
                        .map_err(|_| "Could not store MCP secret".to_owned())?;
                    moved += 1;
                }
            }
        }
        Ok(moved)
    }

    fn restore_redacted_entry(name: &str, entry: &mut Value, path: &std::path::Path) {
        // A redacted read-only value is "keep", never a literal token.
        // Recover it host-side so Override cannot reveal it to the form.
        {
            let (originals, _) = storage::load(
                std::path::Path::new(PI_CONFIG),
                std::path::Path::new("/project"),
                path != std::path::Path::new(PI_CONFIG) && control::is_project_trusted(),
                &[],
            );
            if let Some(original) = originals.iter().find(|r| r.name == *name) {
                let original = config::entry_json(&original.def);
                for field in ["env", "headers"] {
                    if let Some(map) = entry.get_mut(field).and_then(Value::as_object_mut) {
                        for (key, value) in map {
                            if value.as_str() == Some("••••")
                                && let Some(saved) = original.get(field).and_then(|m| m.get(key))
                            {
                                *value = saved.clone();
                            }
                        }
                    }
                }
            }
        }
    }

    // ── config writes ───────────────────────────────────────────────────────

    async fn save_server(name: &str, def: &Value) -> Result<()> {
        Self::mutate(&json!({"scope":"global", "entries": {name: def}}), "save").await?;
        Ok(())
    }

    async fn remove_server_config(name: &str) -> Result<()> {
        Self::mutate(&json!({"scope":"global", "name": name}), "remove").await?;
        Ok(())
    }

    async fn set_enabled(name: &str, enabled: bool) -> Result<()> {
        Self::mutate(
            &json!({"scope":"global", "name":name, "enabled":enabled}),
            "set-enabled",
        )
        .await?;
        Ok(())
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

    fn display_paths(mut value: Value) -> Value {
        fn visit(value: &mut Value, preopens: &[oino_extension::fs::Preopen]) {
            match value {
                Value::String(text) => {
                    for p in preopens {
                        if text.starts_with(&format!("{}/", p.guest_path.trim_end_matches('/'))) {
                            *text = text.replacen(
                                p.guest_path.trim_end_matches('/'),
                                p.host_path.trim_end_matches('/'),
                                1,
                            );
                            break;
                        }
                    }
                }
                Value::Array(items) => {
                    for item in items {
                        visit(item, preopens);
                    }
                }
                Value::Object(map) => {
                    for (_, item) in map {
                        visit(item, preopens);
                    }
                }
                _ => {}
            }
        }
        let preopens = oino_extension::fs::preopens();
        visit(&mut value, &preopens);
        value
    }

    fn servers_json(&self) -> Value {
        let reg = self.registry.borrow();
        let servers: Vec<Value> = reg
            .all()
            .iter()
            .map(|e| {
                let transport = match e.resolved.def.transport {
                    Transport::Stdio => "stdio",
                    Transport::Http => "http",
                };
                let mut entry = config::entry_json(&e.resolved.def);
                storage::redact_entry(&mut entry);
                json!({
                    "name": e.resolved.name,
                    "source": e.resolved.source.as_str(),
                    "sourcePath": e.resolved.source_path,
                    "sourceFile": e.resolved.source_path,
                    "readOnly": e.resolved.source == Source::Acp || e.resolved.source_path.as_deref().is_some_and(|p| p.starts_with("/project/") && p != PROJECT_CONFIG),
                    "entry": entry,
                    "enabled": e.resolved.enabled,
                    "transport": transport,
                    "status": if e.state.error.as_deref().is_some_and(|e| e.contains("needs Autonomy High")) { "needs-autonomy-high" } else { e.state.status.as_str() },
                    "error": e.state.error,
                    "warning": self.fallback_warnings.borrow().get(&e.resolved.name),
                    "toolCount": e.state.tool_count,
                    "tools": e.tool_names,
                    "startedAt": e.state.started_at,
                })
            })
            .collect();
        let mut files = vec![json!({"scope":"global", "path":PI_CONFIG})];
        files.extend(
            storage::PROJECT_FILES
                .iter()
                .map(|p| json!({"scope":"project", "path":format!("/project/{p}")})),
        );
        Self::display_paths(
            json!({"files": files, "trust": {"state": if control::is_project_trusted() { "trusted" } else { "untrusted" }, "path": "/project/"}, "servers": servers, "errors": *self.errors.borrow(), "totalTools": reg.total_tools(), "trusted": control::is_project_trusted(), "paths": {"global": PI_CONFIG, "project": storage::PROJECT_FILES.iter().map(|p| format!("/project/{p}")).collect::<Vec<_>>()}}),
        )
    }

    fn servers_spec(&self) -> Node {
        let reg = self.registry.borrow();
        let items: Vec<ListItem> = reg
            .all()
            .iter()
            .map(|e| {
                let s = &e.state;
                let detail = match (&s.error, s.status) {
                    (Some(err), Status::Error) => format!("{} · {err}", e.resolved.source.as_str()),
                    _ => format!("{} · {} tools", e.resolved.source.as_str(), s.tool_count),
                };
                let tone = status_color(s.status);
                ListItem::new(format!("{} {}", s.status.icon(), s.name))
                    .key(e.resolved.name.clone())
                    .detail(Line::from(detail))
                    .badge(Line::from(s.status.as_str()).fg(tone))
            })
            .collect();
        let list: Node = if items.is_empty() {
            Empty::new()
                .message("No MCP servers configured. Add one from the catalog.")
                .into()
        } else {
            List::new(items).on_activate("select").into()
        };
        let mut children = vec![
            list,
            buttons(&[
                ("Add server…", "add", Some(ButtonVariant::Primary)),
                ("Reload all", "reload", None),
                ("Close", "close", None),
            ]),
        ];
        let errors = self.errors.borrow();
        if !errors.is_empty() {
            children.insert(
                1,
                section("Config problems", Log::new(errors.iter().cloned())),
            );
        }
        children.push(KeyHints::new([("enter", "details"), ("a", "add"), ("q", "close")]).into());
        Layout::default().spacing(1).children(children).into()
    }

    fn server_spec(&self, name: &str) -> Node {
        let reg = self.registry.borrow();
        let Some(e) = reg.get(name) else {
            return Empty::new()
                .message(format!("Server \"{name}\" is gone."))
                .into();
        };
        let def = &e.resolved.def;
        let target = match def.transport {
            Transport::Stdio => format!("{} {}", def.command, def.args.join(" ")),
            Transport::Http => def.url.clone(),
        };
        let mut items = vec![
            ("Status", e.state.status.as_str().to_owned()),
            ("Source", e.resolved.source.as_str().to_owned()),
            (
                "Enabled",
                if e.resolved.enabled { "yes" } else { "no" }.to_owned(),
            ),
            ("Target", target),
            ("Tools", e.state.tool_count.to_string()),
        ];
        if let Some(pid) = e.state.pid {
            items.push(("PID", pid.to_string()));
        }
        if let Some(err) = &e.state.error {
            items.push(("Error", err.clone()));
        }
        let tools: Vec<ListItem> = e
            .tools
            .iter()
            .map(|t| {
                ListItem::new(t.name.clone())
                    .key(t.name.clone())
                    .detail(Line::from(config::tool_exposure(def, &t.mcp_name)))
            })
            .collect();
        let tools_body: Node = if tools.is_empty() {
            Empty::new().message("No tools.").into()
        } else {
            List::new(tools).into()
        };
        let toggle = if e.resolved.enabled {
            "Disable"
        } else {
            "Enable"
        };
        Layout::default()
            .spacing(1)
            .children([
                kv(&items),
                section("Tools", tools_body),
                buttons(&[
                    (toggle, "toggle", None),
                    ("Edit…", "edit", None),
                    ("Reconnect", "reconnect", None),
                    ("Remove", "remove", Some(ButtonVariant::Danger)),
                    ("Back", "back", None),
                ]),
            ])
            .into()
    }

    fn catalog_spec(query: &str) -> Node {
        let cat = catalog::load(
            std::fs::read_to_string(CATALOG_CACHE).ok().as_deref(),
            &now_iso(),
        );
        let items: Vec<ListItem> = catalog::search(&cat, query)
            .into_iter()
            .map(|e| {
                let badge = format!(
                    "{}{} · {}",
                    if e.official { "⭐ " } else { "" },
                    e.scope,
                    e.language
                );
                ListItem::new(e.name.clone())
                    .key(e.id.clone())
                    .detail(Line::from(e.description.clone()))
                    .badge(Line::from(badge))
            })
            .collect();
        let list: Node = if items.is_empty() {
            Empty::new().message("No servers match.").into()
        } else {
            List::new(items).on_activate("pick").into()
        };
        Layout::default()
            .spacing(1)
            .children([
                Input::new("q")
                    .value(query)
                    .placeholder("Search the catalog…")
                    .on_submit("search")
                    .into(),
                Paragraph::new(Line::from(
                    Span::from_string(format!(
                        "{} · {} servers · updated {}",
                        cat.source, cat.total_servers, cat.last_updated
                    ))
                    .dim(),
                ))
                .into(),
                list,
                buttons(&[
                    ("Custom server…", "custom", None),
                    ("Sync catalog", "sync", None),
                    ("Close", "close", None),
                ]),
            ])
            .into()
    }

    fn open_panel(&self, cx: &Ctx, id: &str, title: &str, spec: Node, view: View) -> Result<()> {
        let panel = ui::Panel::new(format!("{EXTENSION_ID}/{id}"))
            .placement(ui::Placement::Overlay)
            .title(title)
            .spec(spec)
            .keys([
                KeyBinding {
                    key: "a".into(),
                    action: Some("add".into()),
                    label: "Add".into(),
                },
                KeyBinding {
                    key: "r".into(),
                    action: Some("reload".into()),
                    label: "Reload".into(),
                },
            ])
            .focus(true)
            .close_on_escape(true)
            .open()?;
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
        let _ = ui::Panel::update(panel).spec(spec).title(title).send();
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
        if let Err(e) = Self::save_server(&name, &entry).await {
            let _ = ui::notify(&format!("MCP server not saved: {e}"), "error");
            return;
        }
        let _ = ui::notify(&format!("MCP server \"{name}\" saved. Connecting…"), "info");
        self.start_named(&name).await;
    }

    async fn start_named(&'static self, name: &str) {
        let (resolved, errors) = self.load().await;
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
                self.stop_server(name, true).await;
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
                if Self::set_enabled(&name, !enabled).await.is_ok() {
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
                            && Self::save_server(&new_name, &def).await.is_ok()
                        {
                            if new_name != name {
                                let _ = Self::remove_server_config(&name).await;
                                self.stop_server(&name, true).await;
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
                    let _ = Self::remove_server_config(&name).await;
                    self.stop_server(&name, true).await;
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
        let (resolved, _) = self.load().await;
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
            } else if ext.active_sessions.get() == 1 {
                // A Settings-only status read had no executable permission context.
                // The first real chat must retry with its own current autonomy.
                ext.reload().await;
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
                        ext.servers_spec(),
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
                        Self::catalog_spec(""),
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
        // Owner 2026-11-05: the app's native MCP page (server table with full errors and
        // per-server actions) instead of a terminal-style overlay list.
        cx.service("mcp.servers", |ext, _p, _caller, _cx| async move {
            if !ext.started.get() {
                ext.start_all().await;
                ext.started.set(true);
            }
            Ok(ext.servers_json())
        });
        cx.service("mcp.catalog", |_ext, p, _caller, _cx| async move {
            let cat = catalog::load(std::fs::read_to_string(CATALOG_CACHE).ok().as_deref(), &now_iso());
            let entries: Vec<Value> = catalog::search(&cat, p.get("query").and_then(Value::as_str).unwrap_or_default())
                .into_iter().map(|e| { let (name, entry) = catalog::template(Some(e)); json!({"id":e.id,"name":name,"label":e.name,"description":e.description,"entry":entry}) }).collect();
            Ok(json!({"entries":entries}))
        });
        cx.service("mcp.save", |ext, p, _caller, _cx| async move {
            match Self::mutate(&p, "save").await {
                Ok(mut result) => {
                    ext.reload().await;
                    result["servers"] = ext.servers_json();
                    Ok(result)
                }
                Err(e) => Ok(json!({"ok": false, "message": e})),
            }
        });
        cx.service("mcp.remove", |ext, p, _caller, _cx| async move {
            match Self::mutate(&p, "remove").await {
                Ok(mut result) => {
                    ext.reload().await;
                    result["servers"] = ext.servers_json();
                    Ok(result)
                }
                Err(e) => Ok(json!({"ok": false, "message": e})),
            }
        });
        cx.service("mcp.set-enabled", |ext, p, _caller, _cx| async move {
            match Self::mutate(&p, "set-enabled").await {
                Ok(mut result) => {
                    ext.reload().await;
                    result["servers"] = ext.servers_json();
                    Ok(result)
                }
                Err(e) => Ok(json!({"ok": false, "message": e})),
            }
        });
        cx.service("mcp.server-action", |ext, p, _caller, _cx| async move {
            let name = p
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            match p.get("action").and_then(Value::as_str).unwrap_or_default() {
                "enable" | "disable" => {
                    let on = p.get("action").and_then(Value::as_str) == Some("enable");
                    if let Err(e) = Self::set_enabled(&name, on).await {
                        return Ok(json!({"ok": false, "message": e.to_string()}));
                    }
                    ext.start_named(&name).await;
                }
                "reconnect" => ext.start_named(&name).await,
                "reload" => ext.reload().await,
                "sync" => ext.sync().await,
                other => {
                    return Ok(json!({"ok": false, "message": format!("unknown action {other}")}));
                }
            }
            Ok(json!({"ok": true, "servers": ext.servers_json()}))
        });
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

    #[test]
    fn catalog_announcements_do_not_repeat() {
        let ext = Mcp::default();
        assert!(ext.announced_unavailable("s").is_none());
        let tools = vec!["a".to_owned(), "b".to_owned()];
        assert!(ext.announced_available("s", &tools).is_some());
        // A restart that brings the same tool set announces nothing.
        assert!(ext.announced_available("s", &tools).is_none());
        assert!(ext.announced_available("s", &tools).is_none());
        // Withdrawal happens once, so a flapping stop cannot repeat it.
        assert!(ext.announced_unavailable("s").is_some());
        assert!(ext.announced_unavailable("s").is_none());
        // Re-announcing after a withdrawal is new information.
        assert!(ext.announced_available("s", &tools).is_some());
        // A changed set announces only the delta.
        let more = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
        let Some(message) = ext.announced_available("s", &more) else {
            unreachable!()
        };
        assert!(message.contains("Newly available deferred tools: c."));
        assert!(ext.announced_available("s", &more).is_none());
    }
}
