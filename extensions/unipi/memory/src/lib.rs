//! `unipi.memory` — MemPalace-backed cross-session memory (unipi `packages/memory`,
//! unipi-port §3.17).
//!
//! | unipi | here |
//! |---|---|
//! | md files `~/.unipi/memory/<project>/<type>/<id>.md` | `/ext-global/memory/…` ([`files`]) |
//! | MemPalace: daemon `/jobs` → `mempalace mine` → pending journal | [`palace`], [`settings::Journal`] |
//! | warm `mempalace-mcp --read-only` reader, 10 min idle unload | [`reader::Reader`] (process pipe + timer) |
//! | markdown-only mode | [`search::local_search`] |
//! | recall reminder (+ wake-up), save pass `side \| reminder \| off` | `before_agent_start`, `agent_end` (side = nested agent) |
//! | tools `memory_store/search/delete/list`, `global_memory_search/list` | tools (exact names, schemas, texts) |
//! | `/memory …`, `-process`, `-consolidate`, `-search`, `global-memory-search`, `-forget`, `global-memory-list` | commands |
//! | cards, reminders renderers, status `unipi-memory`, info group, move handler, `MEMORY_STORED/DELETED` | renderers, status, services, bus |

#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; this init registers synchronously"
)]

pub mod files;
pub mod palace;
pub mod reader;
pub mod search;
pub mod settings;

use std::cell::{Cell, RefCell};
use std::fmt::Write as _;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt;
use oino_extension::agents::Agent;
use oino_extension::command::CompletionItem;
use oino_extension::prelude::*;
use oino_extension::session::{CustomMessage, Delivery, Provenance, SendOptions};
use oino_extension::sync::SessionMap;
use oino_extension::wit::registry::RendererTarget;
use oino_extension::wit::session as ws;
use unipi_kit::services::{MoveReport, MoveRequest, fanout as fan};
use unipi_kit::settings::Store;
use unipi_kit::topics::{self, MemoryDeleted, MemoryStored, ModuleReady};

use crate::files::{MEMORY_TYPES, Record};
use crate::palace::{Bins, Install, Outcome, Places};
use crate::reader::Reader;
use crate::search::Hit;
use crate::settings::{Config, Journal, NAMESPACE, PendingOp, SaveMode};

/// Recall reminder message type.
pub const RECALL_CUSTOM_TYPE: &str = "unipi-memory-recall-reminder";
/// Save reminder message type.
pub const RETRO_CUSTOM_TYPE: &str = "unipi-memory-retro-reminder";
/// Session card entry type.
pub const CARD_CUSTOM_TYPE: &str = "unipi-memory-session-card";
/// Save pass card entry type.
pub const SAVE_CARD_TYPE: &str = "unipi-memory-save-card";
/// The save pass prompt.
pub const SAVE_PROMPT: &str = "Memory save pass. Review the task that just finished in this conversation. If you learned something non-obvious and durable (decision, pitfall + fix, project pattern, user preference), call memory_search to check for an existing memory, then memory_store (update rather than duplicate). If nothing qualifies, reply exactly NONE. Do not call any other tool.";
const MAX_SAVE_TOOL_CALLS: usize = 8;
const PENDING_TIMER: &str = "memory-pending";
const SAVE_ROLE: &str = "memory-save";

const ROOT: &str = "/ext-global/memory";
pub(crate) const MEMPALACE_GUEST: &str = "/mempalace";
const CONFIG_GUEST: &str = "/mempalace-config";

/// The recall reminder text (unipi `buildMemoryRecallReminder`).
#[must_use]
pub fn recall_reminder(
    project: &str,
    titles: &[String],
    can_search: bool,
    can_store: bool,
    wake_up: Option<&str>,
) -> String {
    let mut lines = vec![
        "## 🧠 Memory System Active".to_owned(),
        String::new(),
        format!(
            "You have {} memories stored for project \"{project}\".",
            titles.len()
        ),
    ];
    if can_search && !titles.is_empty() {
        let list = titles
            .iter()
            .take(20)
            .map(|t| format!("- {t}"))
            .collect::<Vec<_>>()
            .join("\n");
        let extra = if titles.len() > 20 {
            format!("\n... and {} more", titles.len() - 20)
        } else {
            String::new()
        };
        lines.push("**BEFORE starting work**, call `memory_search` with relevant keywords to check for existing context.".into());
        lines.push(String::new());
        lines.push("Available memories:".into());
        lines.push(format!("{list}{extra}"));
    }
    if can_store {
        lines.push(String::new());
        lines.push("**AFTER completing the task**, if you learned something non-obvious,".into());
        lines.push("call `memory_store` to save it for future sessions.".into());
    }
    lines.push(String::new());
    lines.push("Guardrails: read max 10 memory results per search. Update existing memories instead of creating duplicates.".into());
    if let Some(w) = wake_up {
        lines.push(String::new());
        lines.push("---".into());
        lines.push(String::new());
        lines.push(w.to_owned());
    }
    lines.join("\n")
}

/// The branch after the last compaction already carries the reminder.
#[must_use]
pub fn branch_has_recall_reminder(branch: &[Value]) -> bool {
    let last = branch
        .iter()
        .rposition(|e| e.get("type").and_then(Value::as_str) == Some("compaction"));
    branch[last.map_or(0, |i| i + 1)..].iter().any(|e| {
        e.get("type").and_then(Value::as_str) == Some("custom_message")
            && e.get("customType").and_then(Value::as_str) == Some(RECALL_CUSTOM_TYPE)
    })
}

/// Inputs of the side-save gate.
#[derive(Clone, Copy, Debug, Default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors unipi shouldRunSideSave input"
)]
pub struct SaveGate {
    /// Write setting.
    pub write: bool,
    /// Session write override.
    pub write_override: Option<bool>,
    /// agent-hooks.json enabled.
    pub hooks: bool,
    /// `memory_store` active.
    pub store_active: bool,
    /// Tool calls in the run.
    pub tool_calls: usize,
    /// Any edit/write.
    pub had_write: bool,
    /// The agent stored/deleted itself.
    pub stored: bool,
    /// A pass is running.
    pub running: bool,
}

/// unipi `shouldRunSideSave`.
#[must_use]
pub fn should_run_side_save(g: SaveGate) -> bool {
    g.write
        && g.write_override != Some(false)
        && g.hooks
        && g.store_active
        && !g.stored
        && (g.tool_calls >= 3 || g.had_write)
        && !g.running
}

/// Wake-up text: `None` = not computed yet, `Some(None)` = none.
#[allow(clippy::option_option, reason = "pending vs absent wake-up")]
type WakeUp = Option<Option<String>>;

#[derive(Default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "per-session flags of unipi memory"
)]
struct Sess {
    recall_done: bool,
    store_done: bool,
    recall_override: Option<bool>,
    write_override: Option<bool>,
    card_done: bool,
    retro_queued: bool,
    run_tool_calls: usize,
    run_had_write: bool,
    run_stored: bool,
    /// `None` = not computed yet; `Some(None)` = no wake-up text.
    wake_up: WakeUp,
}

/// The extension.
#[derive(Default)]
pub struct Memory {
    settings: Store,
    sessions: SessionMap<Sess>,
    install: RefCell<Option<Install>>,
    /// Resolved binaries, keyed by the `mempalaceBin` value they were probed for.
    bins: RefCell<Option<(String, Bins)>>,
    detected: Cell<bool>,
    reader: RefCell<Option<Rc<Reader>>>,
    save_running: Cell<bool>,
    save_collect: RefCell<Vec<String>>,
    replaying: Cell<bool>,
    neutral: Cell<bool>,
}

fn spawn(fut: impl Future<Output = ()> + 'static) {
    #[cfg(target_arch = "wasm32")]
    oino_extension::wit_bindgen::spawn_local(fut);
    #[cfg(not(target_arch = "wasm32"))]
    drop(fut);
}

pub(crate) fn preopen_host(guest: &str) -> Option<String> {
    oino_extension::fs::preopens()
        .into_iter()
        .find(|p| p.guest_path == guest)
        .map(|p| p.host_path)
}

fn root() -> PathBuf {
    PathBuf::from(ROOT)
}

fn host_of(p: &Path) -> String {
    unipi_kit::paths::host_path(p)
        .map_or_else(|| p.display().to_string(), |h| h.display().to_string())
}

fn places() -> Places {
    let mp = preopen_host(MEMPALACE_GUEST);
    Places {
        memory_root: unipi_kit::paths::host_path(&root()).map(|p| p.display().to_string()),
        palace: mp
            .as_ref()
            .map(|h| format!("{}/palace", h.trim_end_matches('/'))),
        mempalace_guest: mp.map(|_| PathBuf::from(MEMPALACE_GUEST)),
    }
}

fn config_dir() -> Option<PathBuf> {
    preopen_host(CONFIG_GUEST).map(|_| PathBuf::from(CONFIG_GUEST))
}

/// Guest path for a host path inside the memory root.
fn guest_of_host(host: &str) -> Option<String> {
    let h = places().memory_root?;
    let rest = host.strip_prefix(&format!("{}/", h.trim_end_matches('/')))?;
    Some(format!("{ROOT}/{rest}"))
}

fn now_iso() -> String {
    unipi_kit::fmt::iso_from_ms(timers::now_ms())
}

fn text_result(text: String, details: Value) -> ToolResult {
    ToolResult::text(text).with_details(details)
}

fn provenance(title: &str, synopsis: &str) -> Provenance {
    Provenance {
        source: "Memory".into(),
        title: Some(title.into()),
        synopsis: Some(synopsis.into()),
        severity: None,
    }
}

impl Memory {
    fn cfg(&self) -> Config {
        settings::read(&self.settings.get(NAMESPACE))
    }

    /// `mempalace` / `mempalace-mcp`: the configured path when it runs, else `PATH`.
    async fn bins(&self) -> Bins {
        let configured = self.cfg().mempalace_bin;
        if let Some((key, b)) = self.bins.borrow().as_ref()
            && *key == configured
        {
            return b.clone();
        }
        let b = Bins::resolve(&configured).await;
        *self.bins.borrow_mut() = Some((configured, b.clone()));
        b
    }

    fn debug(&self, line: &str) {
        if !self.cfg().debug {
            return;
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(unipi_kit::paths::logs("memory.log"))
        {
            let _ = IoWrite::write_fmt(&mut f, format_args!("{} {line}\n", now_iso()));
        }
    }

    fn palace_mode(&self) -> bool {
        self.install.borrow().as_ref().is_some_and(|i| {
            palace::compare_versions(&i.version, palace::MIN_MEMPALACE) != std::cmp::Ordering::Less
        })
    }

    fn install_issue(&self) -> Option<String> {
        match self.install.borrow().as_ref() {
            None => Some("MemPalace isn't installed — install uv (https://docs.astral.sh/uv/) for semantic search".into()),
            Some(i) if !self.palace_mode() && i.rust => Some(format!(
                "MemPalace {} (Rust port) is too old — update the mempalace-rs build for semantic search",
                i.version
            )),
            Some(i) if !self.palace_mode() => Some(format!("MemPalace {} is too old — run `uv tool upgrade mempalace` for semantic search", i.version)),
            Some(_) => None,
        }
    }

    async fn reader(&self) -> Option<Rc<Reader>> {
        if !self.palace_mode() {
            return None;
        }
        let bins = self.bins().await;
        let mut r = self.reader.borrow_mut();
        Some(Rc::clone(r.get_or_insert_with(|| {
            Rc::new(Reader::new(
                places().palace.as_deref(),
                true,
                reader::READER_IDLE_MS,
                bins.mcp,
            ))
        })))
    }

    async fn detect(&self) {
        if self.detected.get() {
            return;
        }
        self.detected.set(true);
        let cache = root().join(".mempalace-install");
        let mut bins = self.bins().await;
        let mut i = palace::ensure_install(&cache, &bins);
        if i.is_none() && palace::uv_install().await {
            bins = Bins::resolve(&self.cfg().mempalace_bin).await;
            *self.bins.borrow_mut() = Some((self.cfg().mempalace_bin, bins.clone()));
            i = palace::ensure_install(&cache, &bins);
        }
        *self.install.borrow_mut() = i;
    }

    fn journal_enqueue(kind: &str, file: &str, project: &str, id: &str, held_by: Option<String>) {
        Journal(&root()).enqueue(PendingOp {
            kind: kind.into(),
            file: file.into(),
            project: project.into(),
            id: id.into(),
            enqueued_at: now_iso(),
            held_by,
        });
        timers::set(PENDING_TIMER, Duration::from_secs(120), true);
    }

    async fn store_record(&self, rec: &Record) -> (Outcome, Option<String>) {
        let root = root();
        files::ensure_mempalace_yaml(&root, &rec.project);
        let md = match files::write_memory_file(&root, rec) {
            Ok(p) => p,
            Err(e) => return (Outcome::MarkdownOnly, Some(e.to_string())),
        };
        let md_host = host_of(&md);
        if !self.palace_mode() {
            Self::journal_enqueue("store", &md_host, &rec.project, &rec.id, None);
            return (
                Outcome::MarkdownOnly,
                Some(
                    self.install_issue()
                        .unwrap_or_else(|| "mempalace not installed".into()),
                ),
            );
        }
        let p = places();
        let bins = self.bins().await;
        let auto = self.cfg().auto_start_daemon;
        let stale: Vec<PathBuf> = files::scan_project(&root, &rec.project)
            .into_iter()
            .filter(|m| m.id == rec.id)
            .filter_map(|m| m.file)
            .filter(|f| *f != md)
            .collect();
        let dir_host = host_of(&files::project_dir(&root, &rec.project));
        let out = palace::file_through_daemon(
            &p,
            &bins,
            auto,
            &dir_host,
            std::slice::from_ref(&md_host),
            &rec.project,
            10_000,
        )
        .await;
        for s in stale {
            let sh = host_of(&s);
            let del = palace::delete_through_daemon(&p, &bins, auto, &sh, 10_000).await;
            let _ = std::fs::remove_file(&s);
            if matches!(del.0, Outcome::Filed | Outcome::Queued) {
                continue;
            }
            if palace::delete_via_write_mcp(&p, &bins, &sh).await.is_err() {
                Self::journal_enqueue("delete", &sh, &rec.project, &rec.id, None);
            }
        }
        if matches!(out.0, Outcome::Filed | Outcome::Queued) {
            return out;
        }
        match palace::mine_direct(&p, &bins, &dir_host, &rec.project).await {
            Ok(()) => (Outcome::Filed, None),
            Err((held, err)) => {
                Self::journal_enqueue("store", &md_host, &rec.project, &rec.id, held);
                (Outcome::MarkdownOnly, err.or(out.1))
            }
        }
    }

    async fn delete(&self, project: &str, key: &str) -> (Outcome, bool) {
        let root = root();
        let local = files::scan_project(&root, project)
            .into_iter()
            .find(|m| m.id == key || m.title == key);
        let md = local
            .as_ref()
            .and_then(|l| l.file.clone())
            .unwrap_or_else(|| {
                files::memory_file_path(
                    &root,
                    project,
                    local.as_ref().map_or("summary", |l| l.kind.as_str()),
                    key,
                )
            });
        let found = local.is_some();
        let _ = std::fs::remove_file(&md);
        let md_host = host_of(&md);
        if !self.palace_mode() {
            Self::journal_enqueue("delete", &md_host, project, key, None);
            return (Outcome::MarkdownOnly, found);
        }
        let p = places();
        let bins = self.bins().await;
        let res = palace::delete_through_daemon(
            &p,
            &bins,
            self.cfg().auto_start_daemon,
            &md_host,
            10_000,
        )
        .await;
        if matches!(res.0, Outcome::Filed | Outcome::Queued) {
            return (res.0, found);
        }
        if palace::delete_via_write_mcp(&p, &bins, &md_host)
            .await
            .is_ok()
        {
            (Outcome::Filed, found)
        } else {
            Self::journal_enqueue("delete", &md_host, project, key, None);
            (Outcome::MarkdownOnly, found)
        }
    }

    async fn search(&self, query: &str, limit: usize, project: Option<&str>) -> Vec<Hit> {
        let Some(r) = self.reader().await else {
            return search::local_search(&root(), query, limit, project);
        };
        let raw = r.search(query, (limit * 3).max(10), project).await;
        let roots: Vec<String> = places().memory_root.into_iter().collect();
        search::group_hits(&raw, limit, &roots, &|h| guest_of_host(h))
    }

    fn list(project: &str) -> Vec<Record> {
        let mut v = files::scan_project(&root(), project);
        v.sort_by(|a, b| b.updated.cmp(&a.updated));
        v
    }

    async fn file_pending(&self, op: &PendingOp) -> Outcome {
        if !self.palace_mode() {
            return Outcome::MarkdownOnly;
        }
        let p = places();
        let bins = self.bins().await;
        let auto = self.cfg().auto_start_daemon;
        if op.kind == "store" {
            let dir = host_of(&files::project_dir(&root(), &op.project));
            let r = palace::file_through_daemon(
                &p,
                &bins,
                auto,
                &dir,
                std::slice::from_ref(&op.file),
                &op.project,
                30_000,
            )
            .await;
            if matches!(r.0, Outcome::Filed | Outcome::Queued) {
                return r.0;
            }
            return if palace::mine_direct(&p, &bins, &dir, &op.project)
                .await
                .is_ok()
            {
                Outcome::Filed
            } else {
                Outcome::MarkdownOnly
            };
        }
        let r = palace::delete_through_daemon(&p, &bins, auto, &op.file, 30_000).await;
        if matches!(r.0, Outcome::Filed | Outcome::Queued) {
            return r.0;
        }
        if palace::delete_via_write_mcp(&p, &bins, &op.file)
            .await
            .is_ok()
        {
            Outcome::Filed
        } else {
            Outcome::MarkdownOnly
        }
    }

    async fn replay(&self) {
        if self.replaying.get() {
            return;
        }
        let j = Journal(&root());
        let ops = j.read();
        if ops.is_empty() {
            timers::cancel(PENDING_TIMER);
            return;
        }
        self.replaying.set(true);
        for op in ops {
            if self.file_pending(&op).await == Outcome::Filed {
                j.drop_op(&op);
            }
        }
        self.replaying.set(false);
    }

    fn project(cx: &Ctx) -> String {
        files::project_name(&cx.cwd().display().to_string())
    }

    fn peek(project: &str, title: &str) -> (Option<Record>, Vec<String>) {
        let root = root();
        let exact = files::find_by_title(&root, project, title);
        let similar = files::find_similar(&root, project, title, 0.6)
            .into_iter()
            .filter(|(r, _)| r.title != title && Some(&r.id) != exact.as_ref().map(|e| &e.id))
            .take(3)
            .map(|(r, s)| format!("\"{}\" ({}%)", r.title, (s * 100.0).round()))
            .collect();
        (exact, similar)
    }

    // ── tool bodies ──────────────────────────────────────────────────────────

    async fn tool_store(&self, args: &Value, cx: &Ctx) -> ToolResult {
        let sid = cx.session_id().to_owned();
        self.sessions.with(&sid, |s| s.store_done = true);
        let title = args
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let content = args
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let project = Self::project(cx);
        let (exact, similar) = Self::peek(&project, &title);
        let want = args
            .get("type")
            .and_then(Value::as_str)
            .filter(|t| MEMORY_TYPES.contains(t));
        let kind = want
            .map(ToOwned::to_owned)
            .or_else(|| exact.as_ref().map(|e| e.kind.clone()))
            .unwrap_or_else(|| "summary".into());
        if let Some(e) = &exact
            && e.content.trim() == content.trim()
            && e.kind == kind
        {
            return text_result(
                format!(
                    "⚠️ Memory already exists with this title and content: \"{title}\". Update it with new content instead of duplicating."
                ),
                json!({"action": "duplicate_detected", "id": e.id}),
            );
        }
        let now = now_iso();
        let tags: Vec<String> = args
            .get("tags")
            .and_then(|t| serde_json::from_value(t.clone()).ok())
            .or_else(|| exact.as_ref().map(|e| e.tags.clone()))
            .unwrap_or_default();
        let rec = Record {
            id: exact
                .as_ref()
                .map(|e| e.id.clone())
                .filter(|i| !i.is_empty())
                .unwrap_or_else(|| files::id_from_title(&title)),
            title: title.clone(),
            content,
            tags,
            project: project.clone(),
            kind: kind.clone(),
            created: exact
                .as_ref()
                .map(|e| e.created.clone())
                .filter(|c| !c.is_empty())
                .unwrap_or_else(|| now.clone()),
            updated: now,
            file: None,
        };
        let (outcome, _) = self.store_record(&rec).await;
        let action = if exact.is_some() {
            "updated"
        } else {
            "created"
        };
        unipi_kit::bus::emit(&MemoryStored {
            id: rec.id.clone(),
            title: rec.title.clone(),
            kind: kind.clone(),
            project: project.clone(),
            action: action.into(),
        });
        if cx.child_role() == Some(SAVE_ROLE) {
            self.save_collect.borrow_mut().push(title.clone());
        }
        let mut lines = vec![
            format!(
                "{} memory: {title}",
                if exact.is_some() { "Updated" } else { "Stored" }
            ),
            format!(
                "{project} › {kind} · {}",
                match outcome {
                    Outcome::Filed => "filed to the palace",
                    Outcome::Queued => "queued — the palace will pick it up",
                    Outcome::MarkdownOnly => "markdown only — daemon unreachable",
                }
            ),
        ];
        if !similar.is_empty() {
            lines.push(format!("Similar memories: {}", similar.join(", ")));
        }
        let mut details = json!({"action": action, "id": rec.id, "title": rec.title, "project": project, "type": kind, "outcome": outcome.as_str()});
        if !similar.is_empty() {
            details["similar"] = json!(similar);
        }
        spawn_replay_kick();
        text_result(lines.join("\n"), details)
    }

    async fn tool_search(
        &self,
        query: &str,
        limit: usize,
        project_scope: bool,
        cx: &Ctx,
    ) -> ToolResult {
        self.sessions
            .with(cx.session_id(), |s| s.recall_done = true);
        let project = Self::project(cx);
        let hits = self
            .search(query, limit, project_scope.then_some(project.as_str()))
            .await;
        if hits.is_empty() {
            return text_result(
                format!("No memories found for: \"{query}\""),
                json!({"query": query, "hits": []}),
            );
        }
        let out = hits
            .iter()
            .enumerate()
            .map(|(i, h)| {
                format!(
                    "{}. [{}] **{}** ({} · {})\n   {}",
                    i + 1,
                    h.wing,
                    h.title,
                    h.room,
                    search::source_label(&h.source_label),
                    h.snippet
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        text_result(
            format!("Found {} memories:\n\n{out}", hits.len()),
            json!({"query": query, "hits": hits}),
        )
    }

    async fn tool_delete(&self, args: &Value, cx: &Ctx) -> ToolResult {
        self.sessions.with(cx.session_id(), |s| s.store_done = true);
        let key = args
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                args.get("title")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
            });
        let Some(key) = key.map(ToOwned::to_owned) else {
            return text_result(
                "Provide a title or id to delete.".into(),
                json!({"deleted": false}),
            );
        };
        let project = Self::project(cx);
        let (outcome, found) = self.delete(&project, &key).await;
        if found {
            unipi_kit::bus::emit(&MemoryDeleted {
                id: key.clone(),
                title: key.clone(),
                project,
            });
        }
        spawn_replay_kick();
        text_result(
            if found {
                format!("Deleted memory: {key} ({})", outcome.as_str())
            } else {
                format!("Memory not found: {key}")
            },
            json!({"deleted": found, "id": key, "title": key, "outcome": outcome.as_str()}),
        )
    }

    fn tool_list(&self, cx: &Ctx) -> ToolResult {
        self.sessions
            .with(cx.session_id(), |s| s.recall_done = true);
        let project = Self::project(cx);
        let mems = Self::list(&project);
        if mems.is_empty() {
            return text_result(
                "No memories stored for this project.".into(),
                json!({"memories": []}),
            );
        }
        let out = mems
            .iter()
            .map(|m| format!("- {} ({})", m.title, m.kind))
            .collect::<Vec<_>>()
            .join("\n");
        let details: Vec<Value> = mems
            .iter()
            .map(|m| json!({"project": project, "id": m.id, "title": m.title, "type": m.kind}))
            .collect();
        text_result(
            format!("Project memories ({}):\n\n{out}", mems.len()),
            json!({"memories": details}),
        )
    }

    fn all_projects() -> Vec<(String, Vec<Record>)> {
        let mut grouped: Vec<(String, Vec<Record>)> = Vec::new();
        for (name, _) in files::list_project_dirs(&root()) {
            let p = files::sanitize_project_name(&name);
            if grouped.iter().any(|(g, _)| *g == p) {
                continue;
            }
            let mems = Self::list(&p);
            if !mems.is_empty() {
                grouped.push((p, mems));
            }
        }
        grouped
    }

    fn tool_global_list(&self, cx: &Ctx) -> ToolResult {
        self.sessions
            .with(cx.session_id(), |s| s.recall_done = true);
        let grouped = Self::all_projects();
        if grouped.is_empty() {
            return text_result(
                "No memories stored in any project.".into(),
                json!({"memories": []}),
            );
        }
        let total: usize = grouped.iter().map(|(_, m)| m.len()).sum();
        let mut out = String::new();
        let mut details = Vec::new();
        for (p, ms) in &grouped {
            let _ = write!(
                out,
                "\n**{p}** ({}):\n{}\n",
                ms.len(),
                ms.iter()
                    .map(|m| format!("  - {} ({})", m.title, m.kind))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            details.extend(
                ms.iter()
                    .map(|m| json!({"project": p, "id": m.id, "title": m.title, "type": m.kind})),
            );
        }
        text_result(
            format!(
                "All memories across {} projects ({total} total):{out}",
                grouped.len()
            ),
            json!({"memories": details}),
        )
    }

    // ── session lifecycle ────────────────────────────────────────────────────

    fn write_tools_active(on: bool) {
        let mut active = control::active_tools();
        active.retain(|t| t != "memory_store" && t != "memory_delete");
        if on {
            active.push("memory_store".into());
            active.push("memory_delete".into());
        }
        let _ = control::set_active_tools(&active);
    }

    #[allow(clippy::too_many_lines, reason = "session start upkeep, as in unipi")]
    fn on_session_start(&'static self, cx: &Ctx) {
        self.settings.refresh();
        let sid = cx.session_id().to_owned();
        let branch = session::branch(None).unwrap_or_default();
        self.sessions.with(&sid, |s| {
            *s = Sess {
                recall_done: branch_has_recall_reminder(&branch),
                ..Sess::default()
            };
        });
        let cfg = self.cfg();
        if !cfg.write {
            Self::write_tools_active(false);
        }
        unipi_kit::bus::emit(&ModuleReady {
            name: "@pi-unipi/memory".into(),
            version: host::info().extension_version,
            commands: [
                "memory",
                "memory-process",
                "memory-consolidate",
                "memory-search",
                "global-memory-search",
                "memory-forget",
                "global-memory-list",
            ]
            .map(ToOwned::to_owned)
            .to_vec(),
            tools: [
                "memory_store",
                "memory_search",
                "memory_delete",
                "memory_list",
            ]
            .map(ToOwned::to_owned)
            .to_vec(),
            load_time_ms: None,
        });
        if cx.is_child() {
            return;
        }
        let has_ui = cx.has_ui();
        if has_ui {
            let _ = ui::status("unipi-memory", Some("🧠 …"));
        }
        let project = Self::project(cx);
        spawn(async move {
            self.detect().await;
            if cfg.recall_at_start && cfg.wake_up && self.palace_mode() {
                let bins = self.bins().await;
                let w = palace::wake_up(&places(), &bins, &project)
                    .await
                    .map(|t| t.chars().take(3500).collect::<String>());
                self.sessions.with(&sid, |s| s.wake_up = Some(w));
            } else {
                self.sessions.with(&sid, |s| s.wake_up = Some(None));
            }
            if has_ui {
                let count = Self::list(&project).len();
                let pend = Journal(&root()).count();
                let status = format!(
                    "🧠 {count} mem{}",
                    if pend > 0 {
                        format!(" +{pend}⧗")
                    } else {
                        String::new()
                    }
                );
                let _ = ui::status("unipi-memory", Some(&status));
                if !self
                    .sessions
                    .with(&sid, |s| std::mem::replace(&mut s.card_done, true))
                {
                    let wake = self.sessions.with(&sid, |s| s.wake_up.clone().flatten());
                    let mut card = json!({"project": project, "count": count});
                    if let Some(w) = wake {
                        card["wakeUp"] = json!(w);
                    }
                    if let Some(i) = self.install_issue() {
                        card["issue"] = json!(i);
                    }
                    let _ = session::append_entry(CARD_CUSTOM_TYPE, Some(&card));
                }
            }
            self.replay().await;
            if Journal(&root()).count() > 0 {
                timers::set(PENDING_TIMER, Duration::from_secs(120), true);
            }
            let install = self.install.borrow().clone();
            if let Some(i) = install.filter(|_| cfg.auto_update) {
                let bins = self.bins().await;
                let (updated, cur, latest, _) = palace::maybe_auto_update(
                    &places(),
                    &bins,
                    &root().join(".mempalace-update"),
                    &i,
                    false,
                )
                .await;
                if updated {
                    unipi_kit::bus::emit_raw(
                        topics::UPDATE_APPLIED,
                        &json!({"previousVersion": cur, "newVersion": latest.unwrap_or_default()}),
                    );
                    if let Some(fresh) =
                        palace::refresh_version(&bins, &root().join(".mempalace-install")).await
                    {
                        *self.install.borrow_mut() = Some(fresh);
                    }
                }
            }
        });
    }

    async fn before_agent_start(&self, cx: &Ctx) -> Option<Value> {
        let sid = cx.session_id().to_owned();
        self.sessions.with(&sid, |s| s.retro_queued = false);
        if self.sessions.with(&sid, |s| s.recall_done) {
            return None;
        }
        let cfg = self.cfg();
        let override_off = self
            .sessions
            .with(&sid, |s| s.recall_override == Some(false));
        if !cfg.recall_at_start
            || override_off
            || !settings::agent_hooks_enabled(config_dir().as_deref())
        {
            self.sessions.with(&sid, |s| s.recall_done = true);
            return None;
        }
        let active = control::active_tools();
        let can_search = active
            .iter()
            .any(|t| t == "memory_search" || t == "global_memory_search");
        let can_store = active.iter().any(|t| t == "memory_store");
        if !can_search && !can_store {
            self.sessions.with(&sid, |s| {
                s.recall_done = true;
                s.store_done = true;
            });
            return None;
        }
        let project = Self::project(cx);
        let titles: Vec<String> = files::scan_project(&root(), &project)
            .into_iter()
            .map(|r| r.title)
            .collect();
        if titles.is_empty() && !can_store {
            self.sessions.with(&sid, |s| s.recall_done = true);
            return None;
        }
        if !can_store {
            self.sessions.with(&sid, |s| s.store_done = true);
        }
        let mut wake = None;
        if cfg.wake_up {
            for _ in 0..10 {
                if let Some(w) = self.sessions.with(&sid, |s| s.wake_up.clone()) {
                    wake = w;
                    break;
                }
                timers::sleep(Duration::from_millis(100)).await;
            }
        }
        self.sessions.with(&sid, |s| s.recall_done = true);
        Some(json!({"message": {
            "customType": RECALL_CUSTOM_TYPE,
            "content": recall_reminder(&project, &titles, can_search, can_store, wake.as_deref()),
            "display": true,
            "details": {"oinoProvenance": {"source": "Memory", "title": "Memory recall", "synopsis": "Recalled at session start"}},
        }}))
    }

    async fn on_agent_end(&'static self, cx: &Ctx) {
        let cfg = self.cfg();
        let sid = cx.session_id().to_owned();
        if cx.is_child() {
            return;
        }
        let store_active = control::active_tools().iter().any(|t| t == "memory_store");
        let hooks = settings::agent_hooks_enabled(config_dir().as_deref());
        match cfg.save_mode {
            SaveMode::Off => {}
            SaveMode::Reminder => {
                let (store_done, recall_done, queued, w_override) = self.sessions.with(&sid, |s| {
                    (
                        s.store_done,
                        s.recall_done,
                        s.retro_queued,
                        s.write_override,
                    )
                });
                if store_done
                    || !recall_done
                    || queued
                    || !cfg.write
                    || w_override == Some(false)
                    || !hooks
                    || !store_active
                {
                    return;
                }
                self.sessions.with(&sid, |s| s.retro_queued = true);
                let _ = session::send_message(
                    &CustomMessage {
                        custom_type: RETRO_CUSTOM_TYPE.into(),
                        content: json!("**🧠 Memory reminder:** If you learned something non-obvious in this task, call `memory_store` to save it as a memory for future sessions. Update existing memories instead of creating duplicates."),
                        display: true,
                        details: None,
                    },
                    SendOptions { trigger_turn: false, deliver_as: Some(Delivery::NextTurn), provenance: Some(provenance("Memory save?", "Save reminder after a finished run")) },
                )
                .await;
            }
            SaveMode::Side => {
                let g = self.sessions.with(&sid, |s| SaveGate {
                    write: cfg.write,
                    write_override: s.write_override,
                    hooks,
                    store_active,
                    tool_calls: s.run_tool_calls,
                    had_write: s.run_had_write,
                    stored: s.run_stored,
                    running: self.save_running.get(),
                });
                if !should_run_side_save(g) {
                    return;
                }
                self.save_running.set(true);
                self.save_collect.borrow_mut().clear();
                spawn(async move {
                    let outcome = self.side_save().await;
                    let titles = std::mem::take(&mut *self.save_collect.borrow_mut());
                    self.debug(&format!(
                        "save: stored=[{}]{}",
                        titles.join(", "),
                        outcome
                            .as_ref()
                            .err()
                            .map(|e| format!(" error={e}"))
                            .unwrap_or_default()
                    ));
                    if !titles.is_empty() {
                        self.sessions.with(&sid, |s| s.store_done = true);
                        let mut card = json!({"titles": titles});
                        if let Ok(Some(u)) = outcome {
                            card["usage"] = u;
                        }
                        let _ = session::append_entry(SAVE_CARD_TYPE, Some(&card));
                    }
                    self.save_running.set(false);
                });
            }
        }
    }

    /// The side save pass: a nested agent seeded with the branch, memory tools only.
    async fn side_save(&self) -> std::result::Result<Option<Value>, String> {
        let spec = json!({
            "label": "memory save",
            "tools": {"builtin": [], "extensionTools": ["memory_store", "memory_search", "memory_list", "global_memory_search", "global_memory_list"], "deny": ["memory_delete"]},
            "extensions": {"only": ["unipi.memory"]},
            "seed": "branchCopy",
            "persistence": "memory",
            "childRole": SAVE_ROLE,
            "ui": "none",
            "maxDepth": 1,
            "thinking": control::thinking_level(),
        });
        let agent = Agent::spawn(&spec).await.map_err(|e| e.to_string())?;
        let mut events = agent.events();
        agent.prompt(SAVE_PROMPT).await.map_err(|e| e.to_string())?;
        let mut calls = 0;
        {
            let idle = agent.wait_idle();
            futures::pin_mut!(idle);
            loop {
                let next = events.next();
                futures::pin_mut!(next);
                match futures::future::select(idle.as_mut(), next).await {
                    futures::future::Either::Left(_) => break,
                    futures::future::Either::Right((None, _)) => {
                        let _ = idle.await;
                        break;
                    }
                    futures::future::Either::Right((Some(ev), _)) => {
                        if ev.get("type").and_then(Value::as_str) == Some("tool_execution_start") {
                            calls += 1;
                            if calls >= MAX_SAVE_TOOL_CALLS {
                                agent.abort();
                            }
                        }
                    }
                }
            }
        }
        let usage = agent.usage().ok().and_then(|u| u.get("tokens").cloned().or(Some(u))).map(|u| {
            json!({"input": u.get("input").and_then(Value::as_u64).unwrap_or(0), "cacheRead": u.get("cacheRead").and_then(Value::as_u64).unwrap_or(0), "cacheWrite": u.get("cacheWrite").and_then(Value::as_u64).unwrap_or(0), "output": u.get("output").and_then(Value::as_u64).unwrap_or(0)})
        });
        agent.close(1_000).await;
        Ok(usage)
    }

    // ── commands ─────────────────────────────────────────────────────────────

    async fn status_text(&self, cx: &Ctx) -> String {
        self.detect().await;
        let cfg = self.cfg();
        let (daemon_up, busy) = palace::probe_daemon(&places(), 300).await;
        let sid = cx.session_id().to_owned();
        let (ro, wo) = self
            .sessions
            .with(&sid, |s| (s.recall_override, s.write_override));
        let project = Self::project(cx);
        let pending = Journal(&root()).read();
        let reader_line = match self.reader.borrow().as_ref() {
            _ if !self.palace_mode() => "none (markdown-only)".to_owned(),
            Some(r) if r.pid().is_some() => format!("warm (pid {})", r.pid().unwrap_or(0)),
            _ => "idle — starts on the next search".to_owned(),
        };
        let install = self.install.borrow().clone();
        let mut lines = vec![
            format!(
                "backend: {}{}",
                install.as_ref().map_or_else(
                    || "unavailable".to_owned(),
                    |i| format!(
                        "mempalace {} ({})",
                        i.version,
                        if i.rust { "rust" } else { "python" }
                    )
                ),
                if self.palace_mode() {
                    ""
                } else {
                    " (markdown-only)"
                }
            ),
            format!(
                "daemon: {}",
                if daemon_up {
                    format!("up{}", if busy { " (busy)" } else { "" })
                } else {
                    "down".into()
                }
            ),
            format!("reader: {reader_line}"),
            format!(
                "project memories: {} ({project})",
                Self::list(&project).len()
            ),
            format!("pending ops: {}", pending.len()),
        ];
        if let Some(h) = pending.iter().find_map(|o| o.held_by.clone()) {
            lines.push(format!("palace busy: held by {h}"));
        }
        if !cfg.mempalace_bin.is_empty() {
            let bins = self.bins().await;
            lines.push(format!(
                "binary: {} (mempalace-mcp: {})",
                bins.cli,
                bins.mcp.first().cloned().unwrap_or_default()
            ));
        }
        lines.push(format!(
            "switches: recall={} write={} wakeUp={} autoStartDaemon={}",
            cfg.recall_at_start && ro != Some(false),
            cfg.write && wo != Some(false),
            cfg.wake_up,
            cfg.auto_start_daemon
        ));
        if let Some(r) = ro {
            lines.push(format!("session recall override: {r}"));
        }
        if let Some(w) = wo {
            lines.push(format!("session write override: {w}"));
        }
        if let Some(i) = self.install_issue() {
            lines.push(format!("mode: {i}"));
        }
        lines.join("\n")
    }

    async fn memory_command(&self, args: &str, cx: &CommandCtx) {
        let parts: Vec<&str> = args.split_whitespace().collect();
        let sub = parts.first().copied().unwrap_or("status");
        let say = |m: &str, l: &str| {
            if cx.has_ui() {
                let _ = ui::notify(m, l);
            }
        };
        match sub {
            "status" => say(&self.status_text(&cx.cx).await, "info"),
            "recall" | "write" => {
                let Some(on) = parts.get(1).and_then(|v| match *v {
                    "on" => Some(true),
                    "off" => Some(false),
                    _ => None,
                }) else {
                    return say(&format!("Usage: /memory {sub} on|off"), "warning");
                };
                if sub == "recall" {
                    self.sessions
                        .with(cx.session_id(), |s| s.recall_override = Some(on));
                    say(
                        &format!(
                            "Memory recall {} for this session.",
                            if on { "on" } else { "off" }
                        ),
                        "info",
                    );
                } else {
                    self.sessions
                        .with(cx.session_id(), |s| s.write_override = Some(on));
                    Self::write_tools_active(on);
                    say(
                        &format!(
                            "Memory write {} for this session.",
                            if on { "on" } else { "off" }
                        ),
                        "info",
                    );
                }
            }
            _ => say(
                "Usage: /memory status | recall on|off | write on|off",
                "info",
            ),
        }
    }

    // ── services ─────────────────────────────────────────────────────────────

    #[allow(clippy::too_many_lines, reason = "unipi move handler steps")]
    async fn move_handle(&self, req: &MoveRequest) -> MoveReport {
        let old_p = files::project_name(&req.old_root);
        let new_p = files::project_name(&req.new_root);
        let mut log = Vec::new();
        if old_p == new_p {
            return MoveReport { log };
        }
        let root = root();
        let old_dir = files::project_dir(&root, &old_p);
        let existing: Vec<String> = files::scan_project(&root, &new_p)
            .into_iter()
            .map(|r| r.id)
            .collect();
        let mut plans = Vec::new();
        for kind in MEMORY_TYPES {
            let mut fs = Vec::new();
            files::walk_md(&old_dir.join(kind), &mut fs);
            for f in fs {
                if let Some(r) = files::parse_memory_file(&f) {
                    let np = files::memory_file_path(&root, &new_p, &r.kind, &r.id);
                    let conflict = existing.contains(&r.id) || np.exists();
                    plans.push((f, np, r, conflict));
                }
            }
        }
        if plans.is_empty() {
            return MoveReport { log };
        }
        log.push(format!("memory: {old_p} → {new_p}"));
        for (o, n, _, c) in &plans {
            log.push(format!(
                "  • {}{} → {}",
                if *c { "skip (same-id conflict): " } else { "" },
                host_of(o),
                host_of(n)
            ));
        }
        if req.dry_run {
            return MoveReport { log };
        }
        let mut moved = Vec::new();
        for (o, _, r, c) in plans {
            if c {
                continue;
            }
            if req.backup {
                let b = unipi_kit::paths::workspace(&format!(
                    "move-backup/{}",
                    o.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                ));
                let _ = unipi_kit::paths::ensure_parent(&b);
                let _ = std::fs::copy(&o, b);
            }
            let rec = Record {
                project: new_p.clone(),
                file: None,
                ..r
            };
            match files::write_memory_file(&root, &rec) {
                Ok(w) => {
                    log.push(format!("write {} → {}: ok", host_of(&o), host_of(&w)));
                    if std::fs::remove_file(&o).is_ok() {
                        moved.push((host_of(&o), host_of(&w)));
                    }
                }
                Err(e) => log.push(format!("write {}: failed: {e}", host_of(&o))),
            }
        }
        for kind in MEMORY_TYPES {
            let _ = std::fs::remove_dir(old_dir.join(kind));
        }
        let _ = std::fs::remove_dir(&old_dir);
        if moved.is_empty() {
            return MoveReport { log };
        }
        files::ensure_mempalace_yaml(&root, &new_p);
        self.detect().await;
        if !self.palace_mode() {
            log.push("reindex: skipped: not installed".into());
            return MoveReport { log };
        }
        let p = places();
        let bins = self.bins().await;
        let dir = host_of(&files::project_dir(&root, &new_p));
        let news: Vec<String> = moved.iter().map(|(_, n)| n.clone()).collect();
        let r = palace::file_through_daemon(
            &p,
            &bins,
            self.cfg().auto_start_daemon,
            &dir,
            &news,
            &new_p,
            30_000,
        )
        .await;
        let filed = match r.0 {
            Outcome::Filed => true,
            Outcome::Queued => false,
            Outcome::MarkdownOnly => palace::mine_direct(&p, &bins, &dir, &new_p).await.is_ok(),
        };
        log.push(format!(
            "reindex: {}",
            if filed { "filed" } else { "failed" }
        ));
        if !filed {
            log.push("delete-old-source: skipped: reindex not filed".into());
            return MoveReport { log };
        }
        for (old, _) in moved {
            let d = palace::delete_through_daemon(
                &p,
                &bins,
                self.cfg().auto_start_daemon,
                &old,
                10_000,
            )
            .await;
            let res = if matches!(d.0, Outcome::Filed | Outcome::Queued) {
                d.0.as_str().to_owned()
            } else if palace::delete_via_write_mcp(&p, &bins, &old).await.is_ok() {
                "filed (direct)".into()
            } else {
                "failed".into()
            };
            log.push(format!("delete-old-source {old}: {res}"));
        }
        MoveReport { log }
    }

    fn info_group(&self, cx: &Ctx) -> Value {
        let cfg = self.cfg();
        let project = Self::project(cx);
        let mems = Self::list(&project);
        let mut total = 0;
        let mut all = Vec::new();
        files::walk_md(&root(), &mut all);
        total += all.len();
        let mut types = serde_json::Map::new();
        for m in &mems {
            let e = types.entry(m.kind.clone()).or_insert(json!(0));
            *e = json!(e.as_u64().unwrap_or(0) + 1);
        }
        let (ro, wo) = self
            .sessions
            .with(cx.session_id(), |s| (s.recall_override, s.write_override));
        let recall = cfg.recall_at_start && ro != Some(false);
        let write = cfg.write && wo != Some(false);
        let pending = Journal(&root()).count();
        json!({
            "id": "memory", "name": "Memory", "icon": "🧠", "priority": 60,
            "config": {"showByDefault": true, "stats": [
                {"id": "projectCount", "label": "Project Memories", "show": true},
                {"id": "totalCount", "label": "Total Memories", "show": true},
                {"id": "recall", "label": "Recall", "show": true},
                {"id": "write", "label": "Write", "show": true},
                {"id": "pending", "label": "Pending Ops", "show": true},
            ]},
            "data": {
                "raw": {"value": "", "raw": {"project": project, "projectCount": mems.len(), "total": total, "types": types, "recall": recall, "write": write, "pending": pending}},
                "projectCount": {"value": mems.len().to_string()},
                "totalCount": {"value": total.to_string()},
                "recall": {"value": if recall { "on" } else { "off" }},
                "write": {"value": if write { "on" } else { "off" }},
                "pending": {"value": pending.to_string()},
            },
        })
    }
}

fn spawn_replay_kick() {
    timers::set(PENDING_TIMER, Duration::from_millis(500), false);
}

// ── renderers (OUS memory cards) ─────────────────────────────────────────────

#[allow(clippy::needless_pass_by_value, reason = "rows are built inline")]
fn rail(rows: Vec<Value>, tone: &str) -> Value {
    json!({"t": "card", "rail": tone, "source": "Memory", "body": {"t": "stack", "direction": "vertical", "children": rows}})
}

#[allow(clippy::needless_pass_by_value, reason = "cells are built inline")]
fn row(left: Value, right: Option<String>) -> Value {
    json!({"t": "stack", "direction": "horizontal", "gap": 2, "children": [{"t": "text", "wrap": false, "value": left}, {"t": "text", "wrap": false, "value": [{"text": right.unwrap_or_default(), "dim": true}]}]})
}

fn outcome_tone(o: &str) -> &'static str {
    match o {
        "filed" => "success",
        "queued" | "" => "warning",
        _ => "error",
    }
}

/// Renders a memory tool result (unipi `renderResult` cards).
#[must_use]
#[allow(clippy::too_many_lines, reason = "one card per memory tool")]
pub fn render_tool(name: &str, d: &Value, text: &str, expanded: bool) -> Value {
    let plural = |n: usize, w: &str| format!("{n} {w}{}", if n == 1 { "" } else { "s" });
    match name {
        "memory_store" => {
            let Some(outcome) = d.get("outcome").and_then(Value::as_str) else {
                return rail(
                    vec![
                        json!({"t": "text", "value": [{"text": "Memory ", "bold": true}, {"text": text.lines().next().unwrap_or_default(), "tone": "warning"}]}),
                    ],
                    "warning",
                );
            };
            let label = match outcome {
                "filed" => "✓ filed",
                "queued" => "⧗ queued",
                _ => "⚠ markdown only",
            };
            let mut rows = vec![row(
                json!([{"text": "Memory ", "bold": true}, {"text": if d.get("action").and_then(Value::as_str) == Some("updated") { "updated " } else { "" }, "dim": true}, {"text": d.get("title").and_then(Value::as_str).unwrap_or_default()}]),
                Some(format!(
                    "{label} {} › {}",
                    d.get("project").and_then(Value::as_str).unwrap_or_default(),
                    d.get("type").and_then(Value::as_str).unwrap_or_default()
                )),
            )];
            for s in d
                .get("similar")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                rows.push(json!({"t": "text", "value": [{"text": format!("~ similar: {s}"), "dim": true}]}));
            }
            rail(rows, outcome_tone(outcome))
        }
        "memory_search" | "global_memory_search" => {
            let q = d.get("query").and_then(Value::as_str).unwrap_or_default();
            let hits: Vec<Value> = d
                .get("hits")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if hits.is_empty() {
                return rail(
                    vec![row(
                        json!([{"text": "Memory ", "bold": true}, {"text": format!("\"{q}\"")}]),
                        Some("no matches".into()),
                    )],
                    "harness",
                );
            }
            let projects: std::collections::BTreeSet<&str> = hits
                .iter()
                .filter_map(|h| h.get("wing").and_then(Value::as_str))
                .collect();
            let shown = if expanded {
                hits.len()
            } else {
                hits.len().min(5)
            };
            let mut rows = vec![row(
                json!([{"text": "Memory ", "bold": true}, {"text": format!("\"{q}\"")}]),
                Some(format!(
                    "{} · {}",
                    plural(hits.len(), "hit"),
                    plural(projects.len(), "project")
                )),
            )];
            for h in &hits[..shown] {
                rows.push(json!({"t": "stack", "direction": "horizontal", "gap": 1, "children": [
                    {"t": "meter", "value": h.get("score").and_then(Value::as_f64).unwrap_or(0.0), "max": 1, "tone": "harness"},
                    {"t": "text", "wrap": false, "value": h.get("title").and_then(Value::as_str).unwrap_or_default()},
                    {"t": "text", "wrap": false, "value": [{"text": format!("{} › {} · {}", h.get("wing").and_then(Value::as_str).unwrap_or_default(), h.get("room").and_then(Value::as_str).unwrap_or_default(), search::source_label(h.get("sourceLabel").and_then(Value::as_str).unwrap_or_default())), "dim": true}]},
                ]}));
            }
            if hits.len() > shown {
                rows.push(json!({"t": "text", "value": [{"text": format!("… {} more", hits.len() - shown), "dim": true}]}));
            }
            rail(rows, "harness")
        }
        "memory_delete" => {
            let deleted = d.get("deleted") == Some(&Value::Bool(true));
            let outcome = d.get("outcome").and_then(Value::as_str).unwrap_or("filed");
            let right = if deleted {
                match outcome {
                    "filed" => "✓ filed",
                    "queued" => "⧗ queued",
                    _ => "⚠ markdown only",
                }
            } else {
                "not found"
            };
            rail(
                vec![row(
                    json!([{"text": "Memory ", "bold": true}, {"text": "forgot ", "dim": true}, {"text": d.get("title").or_else(|| d.get("id")).and_then(Value::as_str).unwrap_or("memory")}]),
                    Some(right.into()),
                )],
                if deleted {
                    outcome_tone(outcome)
                } else {
                    "warning"
                },
            )
        }
        _ => {
            let global = name == "global_memory_list";
            let mems: Vec<Value> = d
                .get("memories")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let shown = if expanded {
                mems.len()
            } else {
                mems.len().min(8)
            };
            let mut rows = vec![row(
                json!([{"text": "Memory ", "bold": true}, {"text": format!("{} {}", mems.len(), if mems.len() == 1 { "memory" } else { "memories" })}]),
                Some(
                    if global {
                        "all projects"
                    } else {
                        "this project"
                    }
                    .into(),
                ),
            )];
            for m in &mems[..shown] {
                let t = m.get("type").and_then(Value::as_str).unwrap_or_default();
                rows.push(row(
                    json!(m.get("title").and_then(Value::as_str).unwrap_or_default()),
                    Some(if global {
                        format!(
                            "{} › {t}",
                            m.get("project").and_then(Value::as_str).unwrap_or_default()
                        )
                    } else {
                        t.to_owned()
                    }),
                ));
            }
            if mems.len() > shown {
                rows.push(json!({"t": "text", "value": [{"text": format!("… {} more", mems.len() - shown), "dim": true}]}));
            }
            rail(rows, "harness")
        }
    }
}

/// `save pass · input … tokens` line.
#[must_use]
pub fn save_usage_line(u: &Value) -> String {
    let f = |k: &str| unipi_kit::fmt::format_tokens(u.get(k).and_then(Value::as_u64).unwrap_or(0));
    format!(
        "save pass · input {} · cache read {} / write {} · output {} tokens",
        f("input"),
        f("cacheRead"),
        f("cacheWrite"),
        f("output")
    )
}

fn memory_completions(prefix: &str) -> Vec<CompletionItem> {
    let raw = prefix.trim_start();
    for sub in ["recall", "write"] {
        if let Some(rest) = raw.strip_prefix(sub).and_then(|r| r.strip_prefix(' '))
            && !rest.contains(char::is_whitespace)
        {
            return ["on", "off"]
                .iter()
                .filter(|v| v.starts_with(rest))
                .map(|v| CompletionItem {
                    value: format!("{sub} {v}"),
                    label: Some((*v).into()),
                    description: None,
                })
                .collect();
        }
    }
    [
        ("status", "backend, daemon, reader, counts, switches"),
        (
            "recall on",
            "session-only: put memory in front of the agent",
        ),
        ("recall off", "session-only: no start reminder"),
        ("write on", "session-only: memory_store/delete active"),
        ("write off", "session-only: read-only tools"),
    ]
    .iter()
    .filter(|(v, _)| v.starts_with(raw))
    .map(|(v, d)| CompletionItem {
        value: (*v).into(),
        label: Some((*v).into()),
        description: Some((*d).into()),
    })
    .collect()
}

async fn send_harness_user(text: String, title: &str, synopsis: &str) {
    let _ = ws::send_user_message(
        json!(text).to_string(),
        ws::UserMessageOptions {
            deliver_as: Some(Delivery::FollowUp),
            expand_prompt_templates: false,
            provenance: Some(provenance(title, synopsis)),
        },
    )
    .await;
}

impl Extension for Memory {
    #[allow(clippy::too_many_lines, reason = "registration list")]
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        self.settings.declare(NAMESPACE, settings::defaults());
        self.settings.load(cx.settings());
        cx.settings_namespace(&settings::namespace().to_json());
        let neutral = !self.cfg().recall_at_start;
        self.neutral.set(neutral);

        cx.on(EventKind::SessionStart, |ext, _ev, cx| async move {
            ext.on_session_start(&cx);
            Ok(())
        });
        cx.on(EventKind::BeforeAgentStart, |ext, _ev, cx| async move {
            Ok(ext.before_agent_start(&cx).await)
        });
        cx.on(EventKind::AgentStart, |ext, _ev, cx| async move {
            ext.sessions.with(cx.session_id(), |s| {
                s.run_tool_calls = 0;
                s.run_had_write = false;
                s.run_stored = false;
            });
            Ok(())
        });
        cx.on(EventKind::ToolCall, |ext, ev, cx| async move {
            let tool = ev
                .get("toolName")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            ext.sessions.with(cx.session_id(), |s| {
                s.run_tool_calls += 1;
                if tool == "edit" || tool == "write" {
                    s.run_had_write = true;
                }
                if tool == "memory_store" || tool == "memory_delete" {
                    s.run_stored = true;
                }
            });
            Ok(())
        });
        cx.on(EventKind::AgentEnd, |ext, _ev, cx| async move {
            ext.on_agent_end(&cx).await;
            Ok(())
        });
        cx.on(EventKind::SessionCompact, |ext, _ev, cx| async move {
            ext.sessions
                .with(cx.session_id(), |s| s.recall_done = false);
            Ok(())
        });
        cx.on(EventKind::OinoBus, |ext, ev, cx| async move {
            if ev.get("topic").and_then(Value::as_str) == Some(topics::COMPACTOR_COMPACTED) {
                ext.sessions
                    .with(cx.session_id(), |s| s.recall_done = false);
            }
            Ok(())
        });
        bus::subscribe(topics::COMPACTOR_COMPACTED);
        cx.on(EventKind::OinoTimer, |ext, ev, _cx| async move {
            match ev.get("timerId").and_then(Value::as_str) {
                Some(reader::IDLE_TIMER) => {
                    let r = ext.reader.borrow().clone();
                    if let Some(r) = r {
                        r.unload_idle().await;
                    }
                }
                Some(PENDING_TIMER) => ext.replay().await,
                _ => {}
            }
            Ok(())
        });
        cx.on(EventKind::OinoSettingsChanged, |ext, ev, _cx| async move {
            let _ = ext.settings.on_changed(&ev.payload);
            Ok(())
        });
        cx.on(EventKind::SessionShutdown, |ext, _ev, cx| async move {
            ext.sessions.remove(cx.session_id());
            if !cx.is_child() {
                let r = ext.reader.borrow().clone();
                if let Some(r) = r {
                    r.kill().await;
                }
                timers::cancel(PENDING_TIMER);
            }
            Ok(())
        });

        for (t, label) in [
            (RECALL_CUSTOM_TYPE, "Memory recall"),
            (RETRO_CUSTOM_TYPE, "Memory save?"),
        ] {
            cx.renderer(RendererTarget::CustomMessage, t, move |_ext, req, _cx| {
                let m = req.get("data")?;
                let content = m.get("content").and_then(Value::as_str).unwrap_or_default();
                let first: String = content.lines().next().unwrap_or_default().chars().filter(|c| !matches!(c, '*' | '#' | '🧠')).collect();
                Some(rail(vec![json!({"t": "text", "value": [{"text": format!("{label} "), "bold": true}, {"text": first.trim(), "dim": true}]})], "harness"))
            });
        }
        cx.renderer(RendererTarget::CustomEntry, CARD_CUSTOM_TYPE, |ext, req, cx| {
            let e = req.get("data")?;
            let d = e.get("data").unwrap_or(e);
            let cfg = ext.cfg();
            let (ro, wo) = ext.sessions.with(cx.session_id(), |s| (s.recall_override, s.write_override));
            let count = d.get("count").and_then(Value::as_u64).unwrap_or(0);
            let wake = d.get("wakeUp").and_then(Value::as_str);
            let issue = d.get("issue").and_then(Value::as_str);
            let mut rows = vec![
                row(json!([{"text": "Memory palace ", "bold": true}, {"text": d.get("project").and_then(Value::as_str).unwrap_or_default()}]), Some(format!("{count} {}", if count == 1 { "memory" } else { "memories" }))),
                json!({"t": "text", "value": [{"text": format!("recall {} · write {}{}", if cfg.recall_at_start && ro != Some(false) { "on" } else { "off" }, if cfg.write && wo != Some(false) { "on" } else { "off" }, if wake.is_some() { " · wake-up ready (expand)" } else { "" }), "dim": true}]}),
            ];
            if let Some(i) = issue {
                rows.push(json!({"t": "text", "value": [{"text": i, "tone": "warning"}]}));
            }
            if req.get("expanded").and_then(Value::as_bool).unwrap_or(false)
                && let Some(w) = wake
            {
                rows.push(json!({"t": "markdown", "value": w}));
            }
            Some(rail(rows, if issue.is_some() { "warning" } else { "harness" }))
        });
        cx.renderer(RendererTarget::CustomEntry, SAVE_CARD_TYPE, |_ext, req, _cx| {
            let e = req.get("data")?;
            let d = e.get("data").unwrap_or(e);
            let titles: Vec<&str> = d.get("titles")?.as_array()?.iter().filter_map(Value::as_str).collect();
            if titles.is_empty() {
                return None;
            }
            let mut rows = vec![json!({"t": "text", "value": [{"text": "Memory saved ", "bold": true}, {"text": titles.join(", ")}]})];
            if let Some(u) = d.get("usage") {
                rows.push(json!({"t": "text", "value": [{"text": save_usage_line(u), "dim": true}]}));
            }
            Some(rail(rows, "harness"))
        });
        for name in [
            "memory_store",
            "memory_search",
            "global_memory_search",
            "memory_delete",
            "memory_list",
            "global_memory_list",
        ] {
            cx.renderer(RendererTarget::Tool, name, move |_ext, req, _cx| {
                let data = req.get("data")?;
                let result = data.get("result")?;
                let text = result
                    .pointer("/content/0/text")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                Some(render_tool(
                    name,
                    result.get("details").unwrap_or(&Value::Null),
                    text,
                    req.get("expanded")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                ))
            });
        }

        cx.service(fan::MOVE_HANDLE, |ext, payload, _caller, _cx| async move {
            let req: MoveRequest = serde_json::from_value(payload)?;
            Ok(json!(ext.move_handle(&req).await))
        });
        cx.service(fan::INFO_SCREEN_GROUPS, |ext, _p, _caller, cx| async move {
            Ok(ext.info_group(&cx))
        });

        let (search_desc, search_snip, search_guides): (&str, &str, Vec<&str>) = if neutral {
            (
                "Searches memories by keyword across ALL projects — pi memories plus drawers written by other tools (Devin, zcode, diaries). Results show [project] title, score, and snippet. Use scope='project' for current project only.",
                "Search memories when relevant.",
                vec![
                    "memory_search is available when you need past context.",
                    "Use scope='project' when you only want this project's memories.",
                ],
            )
        } else {
            (
                "IMPORTANT: Call BEFORE starting work to check for existing context. Searches memories by keyword across ALL projects — pi memories plus drawers written by other tools (Devin, zcode, diaries). Results show [project] title, score, and snippet. Use scope='project' for current project only.",
                "Search memories for relevant context before starting work.",
                vec![
                    "IMPORTANT: Always call memory_search before making decisions when you suspect past work exists.",
                    "Results include memories from other tools — [project] › room tells you where each came from.",
                    "Use scope='project' when you only want this project's memories.",
                ],
            )
        };
        let store_guides: Vec<&str> = if neutral {
            vec![
                "memory_store saves a memory for future sessions; use it when you learned something worth keeping.",
                "Search for existing similar memories first — update if found, create if not.",
            ]
        } else {
            vec![
                "Save non-obvious findings with memory_store at the end of substantive work.",
                "Search for existing similar memories first — update if found, create if not.",
                "Memory is scoped to the current project. Use for decisions, preferences, patterns, summaries.",
            ]
        };
        let mut store = Tool::<Self>::new("memory_store")
            .label("Store Memory")
            .description("Store or update a memory for cross-session recall — user preferences, project decisions, code patterns, and conversation summaries. Update existing memories instead of creating duplicates.")
            .prompt_snippet("Store a memory for cross-session recall.")
            .parameters(&json!({"type": "object", "required": ["title", "content"], "properties": {
                "title": {"type": "string", "description": "Memory title in <most_important>_<less_important>_<lesser> format (e.g., 'auth_jwt_prefer_refresh_tokens')"},
                "content": {"type": "string", "description": "Full memory content (markdown supported)"},
                "tags": {"type": "array", "items": {"type": "string"}, "description": "Tags for categorization"},
                "type": {"type": "string", "description": "Memory type", "enum": MEMORY_TYPES},
            }}))
            .run(|ext, args: Value, cx| async move { Ok(ext.tool_store(&args, &cx.cx).await) });
        for g in store_guides {
            store = store.guideline(g);
        }
        cx.tool(store);
        let mut s = Tool::<Self>::new("memory_search")
            .label("Search Memory")
            .description(search_desc)
            .prompt_snippet(search_snip)
            .kind(ToolKind::Search)
            .parameters(&json!({"type": "object", "required": ["query"], "properties": {
                "query": {"type": "string", "description": "Search query"},
                "limit": {"type": "number", "description": "Max results (default 10)", "default": 10},
                "scope": {"type": "string", "description": "'all' (default) or 'project'", "enum": ["all", "project"], "default": "all"},
            }}))
            .run(|ext, args: Value, cx| async move {
                let q = args.get("query").and_then(Value::as_str).unwrap_or_default().to_owned();
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "small limits")]
                let limit = args.get("limit").and_then(Value::as_f64).filter(|l| *l > 0.0).map_or(10, |l| l as usize);
                let project = args.get("scope").and_then(Value::as_str) == Some("project");
                Ok(ext.tool_search(&q, limit, project, &cx.cx).await)
            });
        for g in search_guides {
            s = s.guideline(g);
        }
        cx.tool(s);
        cx.tool(
            Tool::<Self>::new("global_memory_search")
                .label("Search All Projects")
                .description("Alias for memory_search with scope='all'.")
                .kind(ToolKind::Search)
                .parameters(&json!({"type": "object", "required": ["query"], "properties": {"query": {"type": "string", "description": "Search query"}, "limit": {"type": "number", "default": 10}}}))
                .run(|ext, args: Value, cx| async move {
                    let q = args.get("query").and_then(Value::as_str).unwrap_or_default().to_owned();
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "small limits")]
                    let limit = args.get("limit").and_then(Value::as_f64).filter(|l| *l > 0.0).map_or(10, |l| l as usize);
                    Ok(ext.tool_search(&q, limit, false, &cx.cx).await)
                }),
        );
        cx.tool(
            Tool::<Self>::new("memory_delete")
                .label("Delete Memory")
                .description("Delete a memory by title or ID from the current project.")
                .prompt_snippet("Delete a memory.")
                .kind(ToolKind::Delete)
                .parameters(&json!({"type": "object", "properties": {"title": {"type": "string", "description": "Memory title to delete"}, "id": {"type": "string", "description": "Memory ID to delete"}}}))
                .run(|ext, args: Value, cx| async move { Ok(ext.tool_delete(&args, &cx.cx).await) }),
        );
        cx.tool(
            Tool::<Self>::new("memory_list")
                .label("List Project Memories")
                .description("List all memories for the current project.")
                .prompt_snippet("List all project memories.")
                .kind(ToolKind::Read)
                .run(|ext, _args: Value, cx| async move { Ok(ext.tool_list(&cx.cx)) }),
        );
        cx.tool(
            Tool::<Self>::new("global_memory_list")
                .label("List All Project Memories")
                .description("List all memories across all projects with project names.")
                .kind(ToolKind::Read)
                .run(|ext, _args: Value, cx| async move { Ok(ext.tool_global_list(&cx.cx)) }),
        );

        cx.command(
            Command::<Self>::new("memory")
                .description("Memory status and session overrides: /memory status|recall on|off|write on|off")
                .run(|ext, args, cx| async move {
                    ext.memory_command(&args, &cx).await;
                    Ok(())
                })
                .complete(|_ext, prefix, _cx| async move { memory_completions(&prefix) }),
        );
        cx.command(Command::<Self>::new("memory-process").description("Analyze text and store extracted memories").run(|_ext, args, cx| async move {
            if args.trim().is_empty() {
                if cx.has_ui() {
                    ui::notify("Usage: /memory-process <text to analyze>", "info")?;
                }
                return Ok(());
            }
            if cx.has_ui() {
                ui::notify("Analyzing text for memories... Use memory_store tool to save.", "info")?;
            }
            send_harness_user(format!("Analyze the following text and extract any memory-worthy items (user preferences, project decisions, code patterns, conversation summaries). For each item found, use the memory_store tool to save it.\n\nText to analyze:\n{args}"), "Extract memories", "Generated from /memory-process").await;
            Ok(())
        }));
        cx.command(Command::<Self>::new("memory-consolidate").description("Consolidate current session into memory").run(|_ext, _args, cx| async move {
            if cx.has_ui() {
                ui::notify("Consolidating session into memory... Use memory_store tool to save insights.", "info")?;
            }
            send_harness_user("Review the current session and identify any memory-worthy items:\n- User preferences discovered\n- Project decisions made\n- Code patterns learned\n- Important context to remember\n\nFor each item, use the memory_store tool to save it with an appropriate title and type.".into(), "Consolidate session", "Generated from /memory-consolidate").await;
            Ok(())
        }));
        for (name, desc, global) in [
            ("memory-search", "Search project memories", false),
            (
                "global-memory-search",
                "Search memories across all projects",
                true,
            ),
        ] {
            cx.command(Command::<Self>::new(name).description(desc).run(
                move |ext, args, cx| async move {
                    let say = |m: &str| {
                        if cx.has_ui() {
                            let _ = ui::notify(m, "info");
                        }
                    };
                    let q = args.trim().to_owned();
                    if q.is_empty() {
                        say(&format!("Usage: /{name} <search term>"));
                        return Ok(());
                    }
                    let project = Memory::project(&cx.cx);
                    let hits = ext
                        .search(&q, 10, (!global).then_some(project.as_str()))
                        .await;
                    if hits.is_empty() {
                        say(&if global {
                            format!("No memories found across projects for: \"{args}\"")
                        } else {
                            format!("No memories found for: \"{args}\"")
                        });
                        return Ok(());
                    }
                    let out = hits
                        .iter()
                        .enumerate()
                        .map(|(i, r)| {
                            if global {
                                format!(
                                    "{}. [{}] {} ({})\n   {}",
                                    i + 1,
                                    r.wing,
                                    r.title,
                                    r.room,
                                    r.snippet
                                )
                            } else {
                                format!("{}. {} ({})\n   {}", i + 1, r.title, r.room, r.snippet)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    say(&if global {
                        format!("Found {} memories across projects:\n\n{out}", hits.len())
                    } else {
                        format!("Found {} memories:\n\n{out}", hits.len())
                    });
                    Ok(())
                },
            ));
        }
        cx.command(
            Command::<Self>::new("memory-forget")
                .description("Delete a memory by title")
                .run(|ext, args, cx| async move {
                    let title = args.trim().to_owned();
                    if title.is_empty() {
                        if cx.has_ui() {
                            ui::notify("Usage: /memory-forget <memory title>", "info")?;
                        }
                        return Ok(());
                    }
                    let project = Memory::project(&cx.cx);
                    let (_, found) = ext.delete(&project, &title).await;
                    if found {
                        unipi_kit::bus::emit(&MemoryDeleted {
                            id: title.clone(),
                            title: title.clone(),
                            project,
                        });
                    }
                    if cx.has_ui() {
                        ui::notify(
                            &if found {
                                format!("Deleted memory: {title}")
                            } else {
                                format!("Memory not found: {title}")
                            },
                            "info",
                        )?;
                    }
                    Ok(())
                }),
        );
        cx.command(
            Command::<Self>::new("global-memory-list")
                .description("List all memories across all projects")
                .run(|_ext, _args, cx| async move {
                    let grouped = Memory::all_projects();
                    let msg = if grouped.is_empty() {
                        "No memories stored in any project.".to_owned()
                    } else {
                        let total: usize = grouped.iter().map(|(_, m)| m.len()).sum();
                        let mut out = String::new();
                        for (p, ms) in &grouped {
                            let _ = write!(out, "\n{p} ({}):\n", ms.len());
                            for m in ms {
                                let _ = writeln!(out, "  - {} ({})", m.title, m.kind);
                            }
                        }
                        format!(
                            "All memories across {} projects ({total} total):{out}",
                            grouped.len()
                        )
                    };
                    if cx.has_ui() {
                        ui::notify(&msg, "info")?;
                    }
                    Ok(())
                }),
        );
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(Memory);

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::assert_is_empty,
    clippy::too_many_lines,
    clippy::float_cmp,
    reason = "test assertions"
)]
mod tests {
    use super::*;

    #[test]
    fn reminder() {
        let titles: Vec<String> = (0..22).map(|i| format!("t{i}")).collect();
        let r = recall_reminder("p", &titles, true, true, Some("WAKE"));
        assert!(r.starts_with(
            "## 🧠 Memory System Active\n\nYou have 22 memories stored for project \"p\"."
        ));
        assert!(r.contains("- t19\n... and 2 more"));
        assert!(r.contains("**AFTER completing the task**"));
        assert!(r.ends_with("---\n\nWAKE"));
        let r = recall_reminder("p", &[], true, false, None);
        assert!(!r.contains("Available memories") && !r.contains("AFTER"));
        let b = vec![
            json!({"type": "custom_message", "customType": RECALL_CUSTOM_TYPE}),
            json!({"type": "compaction"}),
        ];
        assert!(!branch_has_recall_reminder(&b));
        assert!(branch_has_recall_reminder(&b[..1]));
    }

    #[test]
    fn save_gate() {
        let g = SaveGate {
            write: true,
            hooks: true,
            store_active: true,
            tool_calls: 3,
            ..SaveGate::default()
        };
        assert!(should_run_side_save(g));
        assert!(!should_run_side_save(SaveGate { tool_calls: 2, ..g }));
        assert!(should_run_side_save(SaveGate {
            tool_calls: 0,
            had_write: true,
            ..g
        }));
        assert!(!should_run_side_save(SaveGate { stored: true, ..g }));
        assert!(!should_run_side_save(SaveGate { running: true, ..g }));
        assert!(!should_run_side_save(SaveGate {
            write_override: Some(false),
            ..g
        }));
        assert_eq!(
            save_usage_line(
                &json!({"input": 1500, "cacheRead": 20000, "cacheWrite": 0, "output": 12})
            ),
            "save pass · input 1.5k · cache read 20k / write 0 · output 12 tokens"
        );
        assert_eq!(memory_completions("rec").len(), 2);
        assert_eq!(memory_completions("write o").len(), 2);
        assert_eq!(memory_completions("write of")[0].value, "write off");
    }

    #[test]
    fn cards() {
        let c = render_tool(
            "memory_store",
            &json!({"action": "updated", "title": "t", "project": "p", "type": "decision", "outcome": "filed", "similar": ["\"x\" (70%)"]}),
            "",
            false,
        );
        assert_eq!(c["rail"], "success");
        let s = render_tool(
            "memory_search",
            &json!({"query": "q", "hits": [{"title": "a", "wing": "w", "room": "r", "score": 0.5, "sourceLabel": "pi"}]}),
            "",
            false,
        );
        assert!(s.to_string().contains("1 hit · 1 project"));
        let l = render_tool("global_memory_list", &json!({"memories": []}), "", false);
        assert!(l.to_string().contains("all projects"));
    }
}
