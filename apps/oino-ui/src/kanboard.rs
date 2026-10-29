//! `KanboardService` (round-5 #7): the board lives in this process.
//!
//! One per app, lazily created and held in a GPUI global ([`kanboard`]) so the
//! Board page and every session pane's board tab share it. Reads and mutations
//! go through `oino_kanboard`'s commands under the same file locks the
//! `oino-kanboard` CLI binary and the `kanboard.exec` host import take; the
//! notify watcher broadcasts the project slug on every task write, so a view
//! refresh triggered by `subscribe` also picks up CLI and agent writes —
//! no daemon, no HTTP port, no SSE stream, no spawned process.

use std::path::Path;
use std::rc::Rc;

use oino_kanboard::board::Board as Store;
use oino_kanboard::model::{Actor, ChainGate, Status, Task};
use oino_kanboard::service::{self, Watcher};
use oino_kanboard::store::{Layout, Project};
use oino_kanboard::{commands, deps, transitions};
use serde_json::Value;
use tokio::sync::broadcast;

use crate::model::kanban as kb;
use crate::model::ous::{OusNode, Span, Tone};

/// Board lanes in board order (the same six the extension panel rendered;
/// cancelled and archived tasks are not board cards).
const COLUMNS: &[(Status, &str)] = &[
    (Status::Backlog, "Backlog"),
    (Status::Todo, "Todo"),
    (Status::InProgress, "In Progress"),
    (Status::Blocked, "Blocked"),
    (Status::InReview, "In Review"),
    (Status::Done, "Done"),
];

fn status_label(status: Status) -> &'static str {
    COLUMNS
        .iter()
        .chain(&[
            (Status::Cancelled, "Cancelled"),
            (Status::Archived, "Archived"),
        ])
        .find(|(id, _)| *id == status)
        .map_or("?", |(_, label)| *label)
}

/// A human action on the board (the views are the `user` actor).
const ACTOR: Actor = Actor::User;

/// A project's open store data (project + tasks), invalidated on writes.
type Snapshot = std::sync::Arc<(Project, Vec<Task>)>;

/// The board service: layout + change broadcast + the file watcher.
pub struct KanboardService {
    layout: Layout,
    tx: broadcast::Sender<String>,
    cache: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, Snapshot>>>,
    _watcher: Option<Watcher>,
}

impl KanboardService {
    /// Service over the env-resolved board home (`$OINO_KANBOARD_HOME`, else
    /// `<agent dir>/state/unipi.kanboard`), watching `<home>/projects`.
    ///
    /// # Errors
    /// The board home cannot be resolved or the OS refuses the watch.
    pub fn new() -> oino_kanboard::Result<Self> {
        Self::with_layout(&Layout::from_env()?)
    }

    /// Service over an explicit layout (tests, a pinned home).
    ///
    /// # Errors
    /// The OS refuses the watch.
    pub fn with_layout(layout: &Layout) -> oino_kanboard::Result<Self> {
        let (tx, _rx) = broadcast::channel(64);
        let cache = std::sync::Arc::new(std::sync::RwLock::new(std::collections::HashMap::<
            String,
            Snapshot,
        >::new()));
        let watcher = service::watch_projects(layout, {
            let tx = tx.clone();
            let cache = std::sync::Arc::clone(&cache);
            move |slug| {
                if let Ok(mut c) = cache.write() {
                    c.remove(slug);
                }
                let _ = tx.send(slug.to_owned());
            }
        })?;
        Ok(Self {
            layout: layout.clone(),
            tx,
            cache,
            _watcher: Some(watcher),
        })
    }

    /// Board-change stream: a project slug per write burst (CLI, agent, GUI).
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }

    /// The layout this service reads.
    #[must_use]
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Registered projects (the picker's list).
    ///
    /// # Errors
    /// The project registry cannot be read.
    pub fn projects(&self) -> oino_kanboard::Result<Vec<Project>> {
        self.layout.list_projects()
    }

    /// The project registered for `cwd`'s git root.
    ///
    /// # Errors
    /// No project is registered for the directory.
    pub fn project_for_cwd(&self, cwd: &Path) -> oino_kanboard::Result<Project> {
        oino_kanboard::store::resolve_project_in(&self.layout, None, None, cwd)
    }

    /// Registers `cwd`'s git root as a project (the "Set up the board" button).
    ///
    /// # Errors
    /// The project cannot be registered.
    pub fn register(&self, cwd: &Path) -> oino_kanboard::Result<Project> {
        commands::project_add(&self.layout, None, None, None, cwd)?;
        self.project_for_cwd(cwd)
    }

    /// Drops a project's cached snapshot (mutations and watcher events).
    fn invalidate(&self, slug: &str) {
        if let Ok(mut cache) = self.cache.write() {
            cache.remove(slug);
        }
    }

    /// The project's open data, cached until a write (file I/O per *change*,
    /// not per render).
    fn store(&self, slug: &str) -> oino_kanboard::Result<Snapshot> {
        if let Ok(cache) = self.cache.read()
            && let Some(entry) = cache.get(slug)
        {
            return Ok(std::sync::Arc::clone(entry));
        }
        let project = Project::load(&self.layout, slug)?;
        let store = Store::open(&self.layout, project.clone())?;
        let entry: Snapshot = std::sync::Arc::new((project, store.tasks()?));
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(slug.to_owned(), std::sync::Arc::clone(&entry));
        }
        Ok(entry)
    }

    /// The native board snapshot for `slug` — same shape the extension panel's
    /// OUS described, built straight from the task files.
    ///
    /// # Errors
    /// The project or its tasks cannot be read.
    pub fn board(
        &self,
        slug: &str,
        selected: Option<&str>,
        session: Option<&str>,
    ) -> oino_kanboard::Result<kb::Board> {
        let (_project, tasks) = &*self.store(slug)?;
        let by_id = |id: &str| tasks.iter().find(|task| task.id == id).cloned();
        let gate = ChainGate::InReview;
        let mut board = kb::Board {
            header: format!("{slug} · {} tasks", tasks.len()),
            select_action: Some("select".to_owned()),
            ..kb::Board::default()
        };
        for (status, label) in COLUMNS {
            let mut lane_tasks: Vec<&Task> =
                tasks.iter().filter(|task| task.status == *status).collect();
            lane_tasks.sort_by(|a, b| {
                // The todo lane reads in claim order (the next command's pick);
                // every other lane in board order.
                if *status == Status::Todo {
                    b.priority
                        .rank()
                        .cmp(&a.priority.rank())
                        .then_with(|| a.order.cmp(&b.order))
                        .then_with(|| a.id.cmp(&b.id))
                } else {
                    a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id))
                }
            });
            board.lanes.push(kb::Lane {
                id: status.as_str().to_owned(),
                title: (*label).to_owned(),
                cards: lane_tasks
                    .iter()
                    .map(|task| card(task, &by_id, gate, session, selected))
                    .collect(),
            });
        }
        if let Some(task) = selected.and_then(|id| tasks.iter().find(|task| task.id == id)) {
            board.moves = moves(task);
            board.detail = Some(detail(task));
        }
        Ok(board)
    }

    /// One task's detail section (the overlay's body).
    ///
    /// # Errors
    /// The project or task cannot be read.
    pub fn task(&self, slug: &str, id: &str) -> oino_kanboard::Result<Task> {
        let (_project, tasks) = &*self.store(slug)?;
        tasks
            .iter()
            .find(|task| task.id == id)
            .cloned()
            .ok_or_else(|| oino_kanboard::Error::not_found(format!("{id}: no such task")))
    }

    /// `add`: a Backlog task from the native dialog.
    ///
    /// # Errors
    /// The task cannot be written (validation, locks).
    pub fn add_task(
        &self,
        slug: &str,
        title: &str,
        body: Option<&str>,
    ) -> oino_kanboard::Result<Value> {
        let project = Project::load(&self.layout, slug)?;
        let common = commands::Common::new(ACTOR, ChainGate::InReview);
        let out = commands::add(
            &self.layout,
            project,
            &common,
            title,
            body,
            Some(Status::Backlog),
            oino_kanboard::Priority::None,
            &[],
            &[],
            &[],
            false,
        );
        self.invalidate(slug);
        out
    }

    /// `move` as the user; `comment` only for the rare transition that needs one
    /// (`Ok` reply is the command's `--json` payload).
    ///
    /// # Errors
    /// The transition rule refuses (the `rule` error names what a comment or a
    /// different actor would fix).
    pub fn move_task(
        &self,
        slug: &str,
        id: &str,
        to: Status,
        comment: Option<&str>,
    ) -> oino_kanboard::Result<Value> {
        let project = Project::load(&self.layout, slug)?;
        let common = commands::Common::new(ACTOR, ChainGate::InReview);
        let out = commands::move_task(&self.layout, project, &common, id, to, comment);
        self.invalidate(slug);
        out
    }

    /// Any `oino-kanboard` argv in-process (order, note, release — the paths
    /// the buttons do not have a typed call for), scoped to `cwd` and `slug`.
    #[must_use]
    pub fn exec(&self, cwd: &Path, slug: &str, argv: &[&str]) -> Value {
        let request = service::ExecRequest {
            argv: argv.iter().map(ToString::to_string).collect(),
            env: [(
                oino_kanboard::store::PROJECT_ENV.to_owned(),
                slug.to_owned(),
            )]
            .into_iter()
            .collect(),
            cwd: Some(cwd.to_path_buf()),
            stdin: None,
        };
        let out = service::exec(&request);
        self.invalidate(slug);
        out
    }
}

/// The GPUI global holding the one service (`None` before first use / when the
/// board home cannot be resolved — views show the empty state, not a crash).
#[cfg(feature = "views")]
struct Shared(Rc<KanboardService>);

#[cfg(feature = "views")]
impl gpui_kit::Global for Shared {}

/// The app-wide service; created on first call. `Err` results become `None`
/// (the views render their empty state and the reason is logged once).
#[cfg(feature = "views")]
pub fn kanboard(cx: &mut gpui_kit::App) -> Option<Rc<KanboardService>> {
    if let Some(shared) = cx.try_global::<Shared>() {
        return Some(Rc::clone(&shared.0));
    }
    match KanboardService::new() {
        Ok(service) => {
            // The refresh pump: every write burst (GUI mutation, CLI call,
            // agent `start`/`finish`) lands as a project slug on the broadcast
            // and repaints the windows — the board's SSE replacement.
            let mut rx = service.subscribe();
            cx.spawn(async move |cx| {
                while rx.recv().await.is_ok() {
                    cx.update(gpui_kit::App::refresh_windows);
                }
            })
            .detach();
            let shared = Shared(Rc::new(service));
            cx.set_global(shared);
            Some(Rc::clone(&cx.global::<Shared>().0))
        }
        Err(err) => {
            tracing::warn!(target: "oino_board", %err, "kanboard service unavailable");
            None
        }
    }
}

// ── native board model ────────────────────────────────────────────────────

fn claimed_by(task: &Task, session: Option<&str>) -> Option<String> {
    let run = task.run.as_ref()?;
    if session.is_some_and(|session| session == run.session) {
        Some("claimed by this session".to_owned())
    } else if run.session.is_empty() {
        Some("claimed".to_owned())
    } else {
        Some(format!("claimed by {}", run.session))
    }
}

/// The text of the latest move into blocked (the comment `commands::task_json`
/// reports as `blockedReason`).
fn blocked_reason(task: &Task) -> Option<String> {
    (task.status == Status::Blocked).then(|| {
        task.activity
            .iter()
            .rev()
            .find(|entry| entry.text.starts_with("blocked"))
            .map(|entry| {
                entry
                    .text
                    .trim_start_matches("blocked")
                    .trim_start_matches([':', '—', ' '])
                    .to_owned()
            })
            .filter(|text| !text.is_empty())
    })?
}

fn card(
    task: &Task,
    by_id: &dyn Fn(&str) -> Option<Task>,
    gate: ChainGate,
    session: Option<&str>,
    selected: Option<&str>,
) -> kb::Card {
    let detail = claimed_by(task, session)
        .or_else(|| blocked_reason(task))
        .or_else(|| {
            deps::pending_deps_regardless_of_status(task, by_id, gate).map(|blocked| {
                format!(
                    "waits for {}",
                    blocked
                        .pending
                        .iter()
                        .map(|(id, _)| id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
        });
    let tone = if task.status == Status::Blocked {
        Tone::Error
    } else {
        match task.priority {
            oino_kanboard::Priority::Urgent => Tone::Error,
            oino_kanboard::Priority::High => Tone::Warning,
            oino_kanboard::Priority::Medium => Tone::Info,
            oino_kanboard::Priority::Low => Tone::Muted,
            oino_kanboard::Priority::None => Tone::Default,
        }
    };
    let priority = match task.priority {
        oino_kanboard::Priority::None => None,
        other => Some(other.as_str().to_owned()),
    };
    kb::Card {
        id: task.id.clone(),
        title: task.display_title(),
        detail,
        priority,
        tone,
        selected: selected == Some(task.id.as_str()),
    }
}

/// The move buttons a user can take (`moves` of the board model).
fn moves(task: &Task) -> Vec<(String, String)> {
    transitions::allowed_targets(task.status, ACTOR)
        .into_iter()
        .map(|to| {
            (
                format!("→ {}", status_label(to)),
                format!("move:{}", to.as_str()),
            )
        })
        .collect()
}

fn span(text: impl Into<String>) -> Span {
    Span {
        text: text.into(),
        ..Span::default()
    }
}

fn dim(text: &str) -> Span {
    Span {
        text: text.to_owned(),
        dim: true,
        ..Span::default()
    }
}

/// The detail section of a task: kv rows, the body as markdown, the full
/// activity list newest first (#11 — the panel sent only the last line).
fn detail(task: &Task) -> OusNode {
    let mut kv = vec![
        (vec![span("Status")], vec![span(status_label(task.status))]),
        (vec![span("Priority")], vec![span(task.priority.as_str())]),
    ];
    if !task.deps.is_empty() {
        kv.push((vec![span("After")], vec![span(task.deps.join(", "))]));
    }
    if let Some(run) = &task.run
        && !run.session.is_empty()
    {
        kv.push((vec![span("Claimed by")], vec![span(run.session.clone())]));
    }
    let mut children = vec![OusNode::Kv { items: kv }];
    if !task.body.trim().is_empty() {
        children.push(OusNode::Markdown {
            value: task.body.clone(),
        });
    }
    if !task.activity.is_empty() {
        // Full list, newest first (round-5 #11 — no truncation in overlays).
        children.push(OusNode::Stack {
            horizontal: false,
            gap: Some(1.0),
            children: task
                .activity
                .iter()
                .rev()
                .map(|entry| OusNode::Text {
                    value: vec![dim(&format!(
                        "{} [{}] {}",
                        entry.at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                        entry.actor.as_str(),
                        entry.text
                    ))],
                    wrap: true,
                })
                .collect(),
        });
    }
    let buttons: Vec<OusNode> = transitions::allowed_targets(task.status, ACTOR)
        .into_iter()
        .map(|to| OusNode::Button {
            label: format!("→ {}", status_label(to)),
            action_id: format!("move:{}", to.as_str()),
            tone: Tone::Default,
            primary: false,
        })
        .collect();
    if !buttons.is_empty() {
        children.push(OusNode::Stack {
            horizontal: true,
            gap: Some(1.0),
            children: buttons,
        });
    }
    OusNode::Section {
        title: vec![span(format!("{} {}", task.id, task.display_title()))],
        body: Box::new(OusNode::Stack {
            horizontal: false,
            gap: Some(2.0),
            children,
        }),
        collapsible: false,
        collapsed: false,
    }
}
