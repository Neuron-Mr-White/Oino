//! Side frame (design.md §4.20, D4) and bottom frame (D7) contents: Review (`DiffViewer`),
//! Files (tree + code viewer), Plan, Agent, Tree, extension panels, single diffs/outputs;
//! Output and Tasks bottom tabs.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::text::TextView;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    px,
};
use oino_ai::JsonValue;
use oino_client::FrontendApi as _;

use crate::automation::Automatable as _;
use crate::components::badge::badge;
use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::empty::empty_state;
use crate::components::log_view;
use crate::components::menu::{MenuEntry, Placement, menu, popover};
use crate::components::ous::{self, Ous};
use crate::components::tabs::{StripSize, TabSpec, frame_tab_bar, tab_strip};
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::kanban;
use crate::model::ous::Tone;
use crate::model::timeline::{self, FileChange, split_path};
use crate::model::tools::{self, DiffLineKind, ToolKind};
use crate::model::transcript::{RowKind, ToolStatus};
use crate::rt::Tokio;
use crate::views::session_pane::{BottomTab, PaneEvent, SessionPane, SideTab};

/// Lines shown by the read-only code viewer.
const VIEW_LINES: usize = 2000;

/// The Files tab: a lazily expanded read-only tree of the session cwd.
#[derive(Debug, Default)]
pub struct FileTree {
    root: PathBuf,
    expanded: Vec<PathBuf>,
    /// Open file and its first lines.
    pub open: Option<(PathBuf, String)>,
    /// Rows of the last walk and when it ran: the tab renders every frame, and walking the
    /// tree each time put `read_dir` syscalls on the UI thread per frame.
    cache: std::cell::RefCell<Option<(std::time::Instant, Vec<(PathBuf, bool, usize)>)>>,
}

/// How long a directory walk of the Files tab is reused.
const TREE_TTL: std::time::Duration = std::time::Duration::from_secs(2);
/// Bytes read for the code viewer (the first [`VIEW_LINES`] lines of large files).
const VIEW_BYTES: u64 = 512 * 1024;

/// The first `max_lines` lines of `path`, reading at most `max_bytes` (lossy UTF-8).
///
/// # Errors
/// The file cannot be opened or read.
pub fn read_head(path: &Path, max_bytes: u64, max_lines: usize) -> std::io::Result<String> {
    use std::io::Read as _;
    let mut buf = Vec::new();
    std::fs::File::open(path)?
        .take(max_bytes)
        .read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf);
    Ok(text.lines().take(max_lines).collect::<Vec<_>>().join("\n"))
}

impl FileTree {
    /// A tree rooted at `cwd`.
    #[must_use]
    pub fn new(cwd: &str) -> Self {
        Self {
            root: PathBuf::from(cwd),
            expanded: Vec::new(),
            open: None,
            cache: std::cell::RefCell::default(),
        }
    }

    fn entries(dir: &Path) -> Vec<(PathBuf, bool)> {
        let mut out: Vec<(PathBuf, bool)> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .filter(|e| e.file_name() != "target" && e.file_name() != "node_modules")
            .map(|e| {
                let dir = e.file_type().is_ok_and(|t| t.is_dir());
                (e.path(), dir)
            })
            .collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out.truncate(500);
        out
    }

    fn rows(&self) -> Vec<(PathBuf, bool, usize)> {
        if let Ok(cache) = self.cache.try_borrow()
            && let Some((at, rows)) = cache.as_ref()
            && at.elapsed() < TREE_TTL
        {
            return rows.clone();
        }
        let mut out = Vec::new();
        self.walk(&self.root, 0, &mut out);
        if let Ok(mut cache) = self.cache.try_borrow_mut() {
            *cache = Some((std::time::Instant::now(), out.clone()));
        }
        out
    }

    fn walk(&self, dir: &Path, depth: usize, out: &mut Vec<(PathBuf, bool, usize)>) {
        if depth > 8 || out.len() > 2000 {
            return;
        }
        for (path, is_dir) in Self::entries(dir) {
            let open = is_dir && self.expanded.contains(&path);
            out.push((path.clone(), is_dir, depth));
            if open {
                self.walk(&path, depth + 1, out);
            }
        }
    }

    fn toggle(&mut self, path: PathBuf, is_dir: bool) {
        *self.cache.get_mut() = None;
        if is_dir {
            if let Some(i) = self.expanded.iter().position(|p| *p == path) {
                self.expanded.remove(i);
            } else {
                self.expanded.push(path);
            }
        } else {
            // Bounded: a multi-GB log must not be read whole on the UI thread.
            let head = read_head(&path, VIEW_BYTES, VIEW_LINES)
                .unwrap_or_else(|e| format!("Cannot read: {e}"));
            self.open = Some((path, head));
        }
    }
}

/// The `unipi:bg:tasks` sticky topic (`{tasks: [BgTaskSnapshot]}`, background-tasks).
pub const BG_TASKS_TOPIC: &str = "unipi:bg:tasks";

/// One row of the Tasks tab, from a `BgTaskSnapshot`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BgTask {
    /// Task id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Shell command.
    pub command: String,
    /// `running|completed|failed|killed`.
    pub status: String,
    /// Start (epoch ms).
    pub start: i64,
    /// End (epoch ms).
    pub end: Option<i64>,
    /// Exit code.
    pub exit_code: Option<i64>,
    /// Output file.
    pub output_path: String,
    /// Last output lines.
    pub tail: Vec<String>,
}

impl BgTask {
    /// Whether it still runs.
    #[must_use]
    pub fn running(&self) -> bool {
        self.status == "running"
    }

    /// Elapsed milliseconds at `now` (epoch ms).
    #[must_use]
    pub fn elapsed_ms(&self, now: i64) -> u64 {
        u64::try_from(self.end.unwrap_or(now) - self.start).unwrap_or(0)
    }
}

/// Parses the `unipi:bg:tasks` topic (running first, then newest first).
#[must_use]
pub fn bg_tasks(data: &JsonValue) -> Vec<BgTask> {
    let str_of = |v: &JsonValue, k: &str| {
        v.get(k)
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let mut out: Vec<BgTask> = data
        .get("tasks")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| {
            let id = str_of(v, "id");
            if id.is_empty() {
                return None;
            }
            let command = str_of(v, "command");
            let name = Some(str_of(v, "name"))
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| command.clone());
            Some(BgTask {
                id,
                name,
                command,
                status: Some(str_of(v, "status"))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "running".to_owned()),
                start: v.get("startTime").and_then(JsonValue::as_i64).unwrap_or(0),
                end: v.get("endTime").and_then(JsonValue::as_i64),
                exit_code: v.get("exitCode").and_then(JsonValue::as_i64),
                output_path: str_of(v, "outputPath"),
                tail: v
                    .get("outputTail")
                    .and_then(JsonValue::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(JsonValue::as_str)
                    .map(str::to_owned)
                    .collect(),
            })
        })
        .collect();
    out.sort_by(|a, b| b.running().cmp(&a.running()).then(b.start.cmp(&a.start)));
    out
}

/// The last `max_lines` lines of `path`, reading at most its last `max_bytes` (lossy UTF-8).
///
/// # Errors
/// The file cannot be opened or read.
pub fn read_tail(path: &Path, max_bytes: u64, max_lines: usize) -> std::io::Result<Vec<String>> {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let from = len.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(from))?;
    let mut buf = Vec::new();
    file.take(max_bytes).read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.lines().collect();
    if from > 0 && !lines.is_empty() {
        // The first line was cut mid-way.
        lines.remove(0);
    }
    let start = lines.len().saturating_sub(max_lines);
    Ok(lines[start..].iter().map(|l| (*l).to_owned()).collect())
}

/// Bytes of a task log the Logs action reads (the tail).
const TASK_LOG_BYTES: u64 = 256 * 1024;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// A launcher entry.
#[derive(Clone, Debug)]
enum Launch {
    /// Opens (or focuses) a side tab.
    Side(SideTab),
    /// Opens a Terminal tab in the bottom frame.
    Terminal,
}

/// Icon of an extension panel tab (kanboard `kanban`, app-bridge `smartphone`, else `blocks`).
fn panel_icon(id: &str) -> IconName {
    if id.starts_with("unipi.kanboard") {
        IconName::Kanban
    } else if id.starts_with("unipi.app-bridge") {
        IconName::Smartphone
    } else {
        IconName::Blocks
    }
}

/// `(running, word)` of a child agent state.
fn agent_state_word(state: oino_core::agents::AgentRunState) -> (bool, &'static str) {
    use oino_core::agents::AgentRunState;
    match state {
        AgentRunState::Running => (true, "running"),
        AgentRunState::Idle => (false, "idle"),
        AgentRunState::Closed => (false, "done"),
    }
}

/// What the Agent tab shows of a live child agent.
struct AgentView<'a> {
    id: &'a str,
    owner: &'a str,
    state: oino_core::agents::AgentRunState,
    elapsed_ms: u64,
    steps: usize,
    rows: &'a [crate::model::transcript::Row],
}

/// Child rows the Agent tab renders (the tail).
const AGENT_ROWS: usize = 200;

/// One read-only row of a child agent timeline.
fn child_row(
    t: &OinoTheme,
    agent: &str,
    ix: usize,
    row: &crate::model::transcript::Row,
) -> AnyElement {
    crate::automation::probe(
        child_body(t, agent, ix, row),
        crate::automation::Meta::labelled(
            crate::automation::Role::Text,
            crate::components::timeline::row_text(Some(row)),
        )
        .value(row.id.clone()),
    )
    .into_any_element()
}

/// The body of one read-only child row (the Agent tab's timeline).
fn child_body(
    t: &OinoTheme,
    agent: &str,
    ix: usize,
    row: &crate::model::transcript::Row,
) -> AnyElement {
    match &row.kind {
        RowKind::User(u) => div()
            .flex()
            .justify_end()
            .child(
                div()
                    .max_w(gpui_kit::relative(0.85))
                    .px(px(space::S3_5))
                    .py(px(space::S2))
                    .rounded(px(radius::R_BUBBLE))
                    .bg(t.bubble())
                    .text_role(t, TextRole::Prose)
                    .text_color(t.fg())
                    .child(SharedString::from(timeline::excerpt(&u.text, 12, 2000))),
            )
            .into_any_element(),
        RowKind::Assistant(a) if !a.text.trim().is_empty() => div()
            .text_role(t, TextRole::Prose)
            .text_color(t.fg())
            .child(
                TextView::markdown(
                    ElementId::Name(format!("agent-{agent}-{}", row.id).into()),
                    a.text.clone(),
                )
                .selectable(true),
            )
            .into_any_element(),
        RowKind::Assistant(a) => div()
            .text_role(t, TextRole::Caption)
            .text_color(t.thinking())
            .child(if a.streaming { "Thinking…" } else { "" })
            .into_any_element(),
        RowKind::Tool(tool) => {
            let line = timeline::tool_line(tool);
            let subject = match &line.subject {
                timeline::Subject::Path { name, dir } => format!("{dir}{name}"),
                timeline::Subject::Command(c) | timeline::Subject::Text(c) => c.clone(),
                timeline::Subject::None => String::new(),
            };
            let (glyph, color) = match tool.status {
                ToolStatus::Running | ToolStatus::Preparing => {
                    (IconName::LoaderCircle, t.working())
                }
                ToolStatus::Error => (IconName::CircleX, t.danger()),
                ToolStatus::Done => (IconName::CircleCheck, t.fg_subtlest()),
            };
            div()
                .id(ElementId::NamedInteger("agent-tool".into(), ix as u64))
                .flex()
                .items_center()
                .gap(px(space::S1_5))
                .min_h(px(size::TOOL_ROW))
                .px(px(space::S1))
                .rounded(px(radius::R_ROW))
                .child(icon(glyph, size::ICON, color))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_role(t, TextRole::Caption)
                        .weight_medium()
                        .text_color(t.fg())
                        .child(SharedString::from(line.verb)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .mono_text(t, TextRole::Sm)
                        .text_color(t.fg_subtlest())
                        .child(SharedString::from(subject)),
                )
                .when_some(line.meta, |el, m| {
                    el.child(
                        div()
                            .mono_text(t, TextRole::Xs)
                            .text_color(t.fg_subtlest())
                            .child(SharedString::from(m)),
                    )
                })
                .into_any_element()
        }
        RowKind::Bash(b) => div()
            .mono_text(t, TextRole::Sm)
            .text_color(t.fg_subtle())
            .child(SharedString::from(format!("$ {}", b.command)))
            .into_any_element(),
        RowKind::Notice(_, text) => div()
            .text_role(t, TextRole::Sm)
            .text_color(t.fg_subtlest())
            .child(SharedString::from(timeline::excerpt(text, 2, 200)))
            .into_any_element(),
        RowKind::Custom(_) | RowKind::Summary(_) => div().into_any_element(),
    }
}

fn tab_spec(tab: &SideTab, title: &str) -> TabSpec {
    let (glyph, label) = match tab {
        SideTab::Review => (IconName::FilePen, "Review".to_owned()),
        SideTab::Files => (IconName::FolderOpen, "Files".to_owned()),
        SideTab::Plan => (IconName::ListChecks, "Plan".to_owned()),
        SideTab::Agent { title, .. } => (IconName::Bot, title.clone()),
        SideTab::Tree => (IconName::ListTree, "Tree".to_owned()),
        SideTab::Panel(id) => (panel_icon(id), title.to_owned()),
        SideTab::Diff { title, .. } => (IconName::FilePen, split_path(title).0),
        SideTab::Output { .. } => (IconName::SquareTerminal, "Output".to_owned()),
        SideTab::Browser => (IconName::Globe, title.to_owned()),
    };
    TabSpec {
        key: SharedString::from(format!("{tab:?}").chars().take(48).collect::<String>()),
        icon: glyph,
        title: SharedString::from(label),
        closable: tab.closable(),
    }
}

/// Body lines a diff card renders at most.
const DIFF_LINES: usize = 2000;

/// One numbered line of the `DiffViewer`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NumberedLine {
    /// Kind.
    pub kind: DiffLineKind,
    /// Old-file line number.
    pub old: Option<usize>,
    /// New-file line number.
    pub new: Option<usize>,
    /// Text without the `+` / `-` / ` ` marker (hunk headers keep theirs).
    pub text: String,
}

/// `(old start, new start)` of a `@@ -a,b +c,d @@` hunk header.
fn hunk_starts(line: &str) -> Option<(usize, usize)> {
    let mut parts = line.trim_start_matches('@').split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let start = |r: &str| r.split(',').next()?.parse::<usize>().ok();
    Some((start(old)?, start(new)?))
}

/// Numbers the body lines of a unified patch (first `max`; the rest is counted as hidden).
/// Lines before the first hunk header (pi edit previews) stay unnumbered.
#[must_use]
pub fn number_diff(patch: &str, max: usize) -> (Vec<NumberedLine>, usize) {
    let preview = tools::diff_preview(patch, max);
    let (mut old, mut new) = (None::<usize>, None::<usize>);
    let lines = preview
        .lines
        .into_iter()
        .map(|l| {
            let strip = |t: &str| t.get(1..).unwrap_or_default().to_owned();
            match l.kind {
                DiffLineKind::Hunk => {
                    if let Some((o, n)) = hunk_starts(&l.text) {
                        (old, new) = (Some(o), Some(n));
                    }
                    NumberedLine {
                        kind: l.kind,
                        old: None,
                        new: None,
                        text: l.text,
                    }
                }
                DiffLineKind::Added => {
                    let n = new;
                    new = new.map(|v| v + 1);
                    NumberedLine {
                        kind: l.kind,
                        old: None,
                        new: n,
                        text: strip(&l.text),
                    }
                }
                DiffLineKind::Removed => {
                    let o = old;
                    old = old.map(|v| v + 1);
                    NumberedLine {
                        kind: l.kind,
                        old: o,
                        new: None,
                        text: strip(&l.text),
                    }
                }
                DiffLineKind::Context | DiffLineKind::Header => {
                    let (o, n) = (old, new);
                    old = old.map(|v| v + 1);
                    new = new.map(|v| v + 1);
                    NumberedLine {
                        kind: l.kind,
                        old: o,
                        new: n,
                        text: if o.is_some() { strip(&l.text) } else { l.text },
                    }
                }
            }
        })
        .collect();
    (lines, preview.hidden)
}

/// Git status letter of a change: `A` for a new file, `D` for a deletion, else `M`.
fn git_letter(t: &OinoTheme, change: &FileChange) -> (&'static str, gpui_kit::Hsla) {
    let new_file = change
        .patches
        .iter()
        .any(|p| p.contains("--- /dev/null") || p.contains("new file mode"));
    let deleted = change
        .patches
        .iter()
        .any(|p| p.contains("+++ /dev/null") || p.contains("deleted file mode"));
    if new_file {
        ("A", t.git_added())
    } else if deleted {
        ("D", t.git_deleted())
    } else {
        ("M", t.git_modified())
    }
}

/// The diff body: hunk separators, 2 px gutter bars, old / new numbers and `diff_*_bg/_fg`
/// line colours (A2.20 P5).
fn diff_body(t: &OinoTheme, patch: &str) -> AnyElement {
    let (lines, hidden) = number_diff(patch, DIFF_LINES);
    let numbered = lines.iter().any(|l| l.old.is_some() || l.new.is_some());
    let num = |n: Option<usize>| {
        div()
            .w(px(size::ICON_BUTTON))
            .flex_shrink_0()
            .pr(px(space::S1_5))
            .flex()
            .justify_end()
            .text_color(t.fg_subtlest())
            .child(SharedString::from(
                n.map(|n| n.to_string()).unwrap_or_default(),
            ))
    };
    let mut col = log_view::log_text(div().flex().flex_col().min_w_full().py(px(space::S1)), t);
    for line in lines {
        if line.kind == DiffLineKind::Hunk {
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .h(px(size::CONTROL_SM))
                    .my(px(space::HALF))
                    .px(px(space::S3))
                    .bg(t.surface())
                    .text_color(t.fg_subtlest())
                    .whitespace_nowrap()
                    .child(SharedString::from(line.text)),
            );
            continue;
        }
        let (bg, bar, fg) = match line.kind {
            DiffLineKind::Added => (
                Some(t.diff_added_bg()),
                Some(t.diff_added()),
                t.diff_added_fg(),
            ),
            DiffLineKind::Removed => (
                Some(t.diff_removed_bg()),
                Some(t.diff_removed()),
                t.diff_removed_fg(),
            ),
            _ => (None, None, t.fg_subtle()),
        };
        col = col.child(
            div()
                .flex()
                .flex_row()
                .when_some(bg, gpui_kit::Styled::bg)
                .child(
                    div()
                        .w(px(space::HALF))
                        .flex_shrink_0()
                        .when_some(bar, gpui_kit::Styled::bg),
                )
                .when(numbered, |el| el.child(num(line.old)).child(num(line.new)))
                .child(
                    div()
                        .pl(px(space::S2))
                        .pr(px(space::S3))
                        .whitespace_nowrap()
                        .text_color(fg)
                        .child(SharedString::from(line.text)),
                ),
        );
    }
    if hidden > 0 {
        col = col.child(
            div()
                .px(px(space::S3))
                .py(px(space::S1))
                .text_color(t.fg_subtlest())
                .text_role(t, TextRole::Sm)
                .child(format!("… {hidden} more lines")),
        );
    }
    col.into_any_element()
}

/// Per-file card of the `DiffViewer`: radius 16 on surface-1, header row with the git letter,
/// name, dim directory and `+n −m`, then the diff body (scrolls sideways).
fn file_card(t: &OinoTheme, change: &FileChange, key: usize) -> AnyElement {
    let (name, dir) = split_path(&change.path);
    let patch = change.patches.join("\n");
    let (letter, letter_color) = git_letter(t, change);
    div()
        .flex()
        .flex_col()
        .w_full()
        .rounded(px(radius::R_CARD))
        .bg(t.card())
        .border_1()
        .border_color(t.card_border())
        .overflow_hidden()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(space::S2))
                .px(px(space::S3_5))
                .h(px(size::MENU_ROW))
                .border_b_1()
                .border_color(t.row_divider())
                .child(
                    div()
                        .mono_text(t, TextRole::Sm)
                        .weight_semibold()
                        .text_color(letter_color)
                        .child(letter),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_role(t, TextRole::Base)
                        .weight_medium()
                        .text_color(t.fg())
                        .child(SharedString::from(name)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .mono_text(t, TextRole::Sm)
                        .text_color(t.fg_subtlest())
                        .child(SharedString::from(dir)),
                )
                .child(
                    div()
                        .mono_text(t, TextRole::Xs)
                        .text_color(t.success())
                        .child(format!("+{}", change.added)),
                )
                .child(
                    div()
                        .mono_text(t, TextRole::Xs)
                        .text_color(t.danger())
                        .child(format!("−{}", change.removed)),
                ),
        )
        .child(
            div()
                .id(ElementId::NamedInteger("review-file".into(), key as u64))
                .overflow_x_scroll()
                .child(diff_body(t, &patch)),
        )
        .into_any_element()
}

#[cfg(feature = "terminal")]
impl SessionPane {
    fn spawn_terminal(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(
        gpui_kit::Entity<crate::components::terminal::TerminalView>,
        gpui_kit::Subscription,
    )> {
        use crate::components::terminal::{Shell, TerminalEvent, TerminalView};
        use gpui_kit::AppContext as _;
        let shell = match Shell::spawn(Path::new(&self.model.cwd)) {
            Ok(s) => s,
            Err(error) => {
                tracing::warn!(%error, "starting a terminal");
                return None;
            }
        };
        let view = cx.new(|cx| TerminalView::new(shell, cx));
        let sub = cx.subscribe(&view, |this, view, event: &TerminalEvent, cx| match event {
            TerminalEvent::OpenUrl(url) => this.open_link(url, cx),
            TerminalEvent::Exited => {
                for tab in &mut this.terminals {
                    tab.views.retain(|v| v != &view);
                }
                let gone: Vec<u64> = this
                    .terminals
                    .iter()
                    .filter(|t| t.views.is_empty())
                    .map(|t| t.id)
                    .collect();
                for id in gone {
                    this.close_terminal(id, cx);
                }
                cx.notify();
            }
            TerminalEvent::Title(_) => cx.notify(),
        });
        Some((view, sub))
    }

    /// Opens a new Terminal tab in the bottom frame (the `+` button).
    pub fn new_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((view, sub)) = self.spawn_terminal(cx) else {
            cx.emit(PaneEvent::Toast("Could not start a shell".into()));
            return;
        };
        view.update(cx, |v, cx| v.focus(window, cx));
        let id = self.next_terminal;
        self.next_terminal += 1;
        if self.bottom.0.is_empty() {
            // Opened from the launcher before the bottom frame was ever shown.
            self.bottom.0 = vec![BottomTab::Output, BottomTab::Tasks];
        }
        self.terminals
            .push(crate::views::session_pane::TerminalTab::new(id, view, sub));
        self.bottom.0.push(BottomTab::Terminal(id));
        self.bottom.1 = self.bottom.0.len() - 1;
        cx.emit(PaneEvent::ShowBottom);
        cx.notify();
    }

    /// Splits the active Terminal tab (two shells side by side).
    pub fn split_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(BottomTab::Terminal(id)) = self.bottom.0.get(self.bottom.1).cloned() else {
            return;
        };
        if self
            .terminals
            .iter()
            .any(|t| t.id == id && t.views.len() >= 2)
        {
            return;
        }
        let Some((view, sub)) = self.spawn_terminal(cx) else {
            return;
        };
        view.update(cx, |v, cx| v.focus(window, cx));
        if let Some(tab) = self.terminals.iter_mut().find(|t| t.id == id) {
            tab.push(view, sub);
        }
        cx.notify();
    }

    /// Closes Terminal tab `id` (its shells shut down).
    pub fn close_terminal(&mut self, id: u64, cx: &mut Context<Self>) {
        self.terminals.retain(|t| t.id != id);
        self.bottom.0.retain(|b| *b != BottomTab::Terminal(id));
        if self.bottom.1 >= self.bottom.0.len() {
            self.bottom.1 = self.bottom.0.len().saturating_sub(1);
        }
        cx.notify();
    }
}

impl SessionPane {
    /// Opens the bottom frame on the Tasks tab.
    pub fn open_bottom_tasks(&mut self, cx: &mut Context<Self>) {
        if !self.bottom.0.contains(&BottomTab::Tasks) {
            self.bottom.0.push(BottomTab::Tasks);
        }
        self.bottom.1 = self
            .bottom
            .0
            .iter()
            .position(|t| *t == BottomTab::Tasks)
            .unwrap_or(0);
        cx.emit(PaneEvent::ShowBottom);
        cx.notify();
    }

    /// What the `+` launcher and the empty side pane offer (A2.20 P1): Review, Files, Plan,
    /// Tree, Browser, Terminal and the session's `sidePane` extension panels.
    fn launch_items(&self, cx: &App) -> Vec<(IconName, String, Launch)> {
        let mut items: Vec<(IconName, String, Launch)> = vec![
            (
                IconName::FilePen,
                "Review".into(),
                Launch::Side(SideTab::Review),
            ),
            (
                IconName::FolderOpen,
                "Files".into(),
                Launch::Side(SideTab::Files),
            ),
            (
                IconName::ListChecks,
                "Plan".into(),
                Launch::Side(SideTab::Plan),
            ),
            (
                IconName::ListTree,
                "Tree".into(),
                Launch::Side(SideTab::Tree),
            ),
        ];
        if crate::platform::BrowserSupport::get(cx) {
            items.push((
                IconName::Globe,
                "Browser".into(),
                Launch::Side(SideTab::Browser),
            ));
        }
        if cfg!(feature = "terminal") {
            items.push((
                IconName::SquareTerminal,
                "Terminal".into(),
                Launch::Terminal,
            ));
        }
        for (id, title) in self.side_panels() {
            let glyph = panel_icon(&id);
            items.push((glyph, title, Launch::Side(SideTab::Panel(id))));
        }
        items
    }

    /// Runs a launcher entry.
    fn launch(&mut self, item: Launch, window: &mut Window, cx: &mut Context<Self>) {
        match item {
            Launch::Side(tab) => self.open_side(tab, cx),
            Launch::Terminal => {
                #[cfg(feature = "terminal")]
                self.new_terminal(window, cx);
                #[cfg(not(feature = "terminal"))]
                let _ = window;
            }
        }
    }

    /// The `+` launcher: an anchored menu of [`Self::launch_items`] (GA-13).
    fn launcher(&self, t: &OinoTheme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let state = window.use_keyed_state(
            ElementId::Name(format!("side-launcher-{}", self.model.id).into()),
            cx,
            |_, _| false,
        );
        let open = *state.read(cx);
        let items = self.launch_items(cx);
        let entity = cx.entity().downgrade();
        let toggle = state.clone();
        let trigger = Btn::icon_only("side-add", IconName::Plus, "Open a tab")
            .size(BtnSize::Lg)
            .radius(radius::R_FULL)
            .selected(open)
            .on_click(move |_, _, cx| {
                toggle.update(cx, |v, cx| {
                    *v = !*v;
                    cx.notify();
                });
            });
        let pick = state.clone();
        let close = state;
        let t = *t;
        popover(
            trigger,
            open,
            Placement::BelowEnd,
            move || {
                let entries = items
                    .into_iter()
                    .map(|(glyph, label, item)| {
                        let e = entity.clone();
                        let s = pick.clone();
                        MenuEntry::item(label, move |w, cx| {
                            s.update(cx, |v, cx| {
                                *v = false;
                                cx.notify();
                            });
                            let item = item.clone();
                            let _ = e.update(cx, |p, cx| p.launch(item, w, cx));
                        })
                        .icon(glyph)
                    })
                    .collect();
                menu(
                    "side-launcher-menu",
                    &t,
                    size::MENU_MIN + space::S8,
                    entries,
                )
                .into_any_element()
            },
            move |_, cx| {
                close.update(cx, |v, cx| {
                    *v = false;
                    cx.notify();
                });
            },
        )
        .into_any_element()
    }

    /// The dim line under the side header title.
    fn side_subtitle(&self, tab: &SideTab, cx: &App) -> String {
        match tab {
            SideTab::Review => {
                let changes = timeline::file_changes(self.model.transcript.rows());
                if changes.is_empty() {
                    "No changes yet".to_owned()
                } else {
                    let added: usize = changes.iter().map(|c| c.added).sum();
                    let removed: usize = changes.iter().map(|c| c.removed).sum();
                    format!(
                        "{} file{} changed · +{added} −{removed}",
                        changes.len(),
                        if changes.len() == 1 { "" } else { "s" }
                    )
                }
            }
            SideTab::Files => self.model.cwd.clone(),
            SideTab::Plan => "Plan mode".to_owned(),
            SideTab::Agent { call_id, .. } => match self.agent_live(call_id) {
                Some(a) => format!("{} · {}", a.owner, agent_state_word(a.state).1),
                None => "Subagent".to_owned(),
            },
            SideTab::Tree => "Session tree".to_owned(),
            SideTab::Panel(id) => id.split('/').next().unwrap_or(id).to_owned(),
            SideTab::Diff { title, .. } => split_path(title).1,
            SideTab::Output { .. } => "Tool output".to_owned(),
            SideTab::Browser => {
                let _ = cx;
                "Browser".to_owned()
            }
        }
    }

    /// The side header (`DetailHeader` look, 56): icon tile, title and dim sub-line, then the
    /// launcher, close-tab, hide-pane and (CSD) window controls.
    fn side_header(
        &self,
        t: &OinoTheme,
        spec: Option<&TabSpec>,
        subtitle: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (glyph, title) = spec.map_or((IconName::PanelRight, "Side pane".into()), |s| {
            (s.icon, s.title.clone())
        });
        let single_closable = self.side.tabs.len() == 1 && spec.is_some_and(|s| s.closable);
        let launcher = self.launcher(t, window, cx);
        div()
            .flex()
            .items_center()
            .flex_shrink_0()
            .h(px(size::DETAIL_HEAD))
            .pl(px(space::S3))
            .pr(px(space::S2))
            .gap(px(space::S2_5))
            .border_b_1()
            .border_color(t.line_soft())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .flex_shrink_0()
                    .size(px(size::ICON_BUTTON))
                    .rounded(px(radius::R_ROW))
                    .bg(t.chip())
                    .child(icon(glyph, size::ICON, t.fg_subtle())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_role(t, TextRole::Lg)
                            .weight_semibold()
                            .text_color(t.fg())
                            .child(title),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_role(t, TextRole::Sm)
                            .text_color(t.fg_subtlest())
                            .child(SharedString::from(subtitle)),
                    ),
            )
            .child(launcher)
            .when(single_closable, |el| {
                el.child(
                    Btn::icon_only("side-close-tab", IconName::X, "Close tab")
                        .size(BtnSize::Lg)
                        .radius(radius::R_FULL)
                        .on_click(cx.listener(|this, _, _, cx| this.close_side_tab(0, cx))),
                )
            })
            .child(
                Btn::icon_only("side-hide", IconName::PanelRight, "Hide the side pane")
                    .tooltip("Hide the side pane", Some("⌃J"))
                    .size(BtnSize::Lg)
                    .radius(radius::R_FULL)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(PaneEvent::ToggleSide))),
            )
            .when(self.header_state.controls_side, |el| {
                el.child(crate::components::window_chrome::window_controls(
                    t,
                    self.header_state.maximized,
                ))
            })
            .into_any_element()
    }

    /// Closes side tab `i` (panels send `$close`, the browser view is released).
    fn close_side_tab(&mut self, i: usize, cx: &mut Context<Self>) {
        match self.side.tabs.get(i).cloned() {
            Some(SideTab::Panel(id)) => self.close_panel(&id, cx),
            #[cfg(feature = "browser")]
            Some(SideTab::Browser) => {
                self.browser = None;
                self.side.close_at(i);
            }
            _ => {
                self.side.close_at(i);
            }
        }
        cx.notify();
    }

    /// The pill strip, shown only when more than one tab is open.
    fn side_strip(
        &self,
        t: &OinoTheme,
        specs: &[TabSpec],
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if specs.len() < 2 {
            return None;
        }
        let entity = cx.entity().downgrade();
        let e2 = entity.clone();
        let on_select: Rc<dyn Fn(usize, &mut Window, &mut App)> = Rc::new(move |i, _, cx| {
            let _ = entity.update(cx, |p, cx| {
                p.side.active = i;
                cx.notify();
            });
        });
        let on_close: Rc<dyn Fn(usize, &mut Window, &mut App)> = Rc::new(move |i, _, cx| {
            let _ = e2.update(cx, |p, cx| p.close_side_tab(i, cx));
        });
        Some(
            frame_tab_bar(
                "side",
                t,
                specs,
                self.side.active,
                on_select,
                on_close,
                Vec::new(),
            )
            .into_any_element(),
        )
    }

    fn side_specs(&self, cx: &App) -> Vec<TabSpec> {
        self.side
            .tabs
            .iter()
            .map(|tab| {
                let title = match tab {
                    SideTab::Panel(id) => self
                        .model
                        .ui
                        .panel(id)
                        .map(|p| p.title.clone())
                        .unwrap_or_default(),
                    SideTab::Browser => self.browser_title(cx),
                    _ => String::new(),
                };
                tab_spec(tab, &title)
            })
            .collect()
    }

    fn browser_title(&self, cx: &App) -> String {
        #[cfg(feature = "browser")]
        if let Some(b) = &self.browser {
            return b.read(cx).title();
        }
        let _ = cx;
        "Browser".to_owned()
    }

    /// The Browser tab body (creates the view and applies a pending URL).
    fn render_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        #[cfg(feature = "browser")]
        {
            let pending = self.browser_pending.take();
            match (&self.browser, pending) {
                (Some(b), Some(url)) => b.update(cx, |b, cx| b.load(&url, cx)),
                (None, url) => {
                    let url = url.unwrap_or_else(|| "about:blank".to_owned());
                    let view = gpui_kit::AppContext::new(cx, |cx| {
                        crate::components::browser::BrowserView::for_session(
                            &self.model.id,
                            &self.model.cwd,
                            &url,
                            window,
                            cx,
                        )
                    });
                    cx.subscribe(
                        &view,
                        |_, _, _: &crate::components::browser::BrowserEvent, cx| {
                            cx.notify();
                        },
                    )
                    .detach();
                    self.browser = Some(view);
                }
                (Some(_), None) => {}
            }
            if let Some(b) = &self.browser {
                return b.clone().into_any_element();
            }
        }
        let _ = window;
        let t = OinoTheme::get(cx);
        empty_state(
            &t,
            IconName::Globe,
            "No embedded browser in this build",
            None,
            Vec::new(),
        )
        .into_any_element()
    }

    /// The side pane (A2.20 P1): a flush right column with a `DetailHeader`-style header for
    /// the active tab, a pill strip when more than one tab is open, then the tab content
    /// (launcher grid when no tab is open).
    #[allow(clippy::too_many_lines, reason = "one arm per tab kind")]
    pub fn render_side(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = OinoTheme::get(cx);
        // Panels closed by their extension drop their tabs.
        let ui = &self.model.ui;
        self.side.tabs.retain(|tab| match tab {
            SideTab::Panel(id) => ui.panel(id).is_some(),
            _ => true,
        });
        if self.side.active >= self.side.tabs.len() {
            self.side.active = self.side.tabs.len().saturating_sub(1);
        }
        let specs = self.side_specs(cx);
        let subtitle = self
            .side
            .current()
            .map(|tab| self.side_subtitle(tab, cx))
            .unwrap_or_default();
        let header = self.side_header(&t, specs.get(self.side.active), subtitle, window, cx);
        let strip = self.side_strip(&t, &specs, cx);
        if self.side.current() == Some(&SideTab::Browser) {
            // The browser fills the pane edge to edge (no padding / scroll wrapper); its own
            // URL row and take-over strip come from the browser view.
            let body = self.render_browser(window, cx);
            return div()
                .flex()
                .flex_col()
                .size_full()
                .bg(t.window())
                .child(header)
                .children(strip)
                .child(div().flex_1().min_h_0().child(body))
                .into_any_element();
        }
        let mut review: Option<Vec<AnyElement>> = None;
        let body: AnyElement = match self.side.current().cloned() {
            None => self.render_launcher(&t, cx),
            Some(SideTab::Review) => {
                let changes = timeline::file_changes(self.model.transcript.rows());
                if changes.is_empty() {
                    empty_state(
                        &t,
                        IconName::FilePen,
                        "No changes yet",
                        Some("Edits made in this session appear here.".into()),
                        Vec::new(),
                    )
                    .into_any_element()
                } else {
                    let added: usize = changes.iter().map(|c| c.added).sum();
                    let removed: usize = changes.iter().map(|c| c.removed).sum();
                    let cursor = self.diff_cursor;
                    // Flat children of the scroll container so n/p can scroll to a card.
                    review = Some(
                        std::iter::once(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(space::S2))
                                .pb(px(space::S3))
                                .child(crate::components::section_label::label(
                                    &t,
                                    format!(
                                        "{} file{} changed",
                                        changes.len(),
                                        if changes.len() == 1 { "" } else { "s" }
                                    ),
                                ))
                                .child(
                                    div()
                                        .mono_text(&t, TextRole::Xs)
                                        .text_color(t.success())
                                        .child(format!("+{added}")),
                                )
                                .child(
                                    div()
                                        .mono_text(&t, TextRole::Xs)
                                        .text_color(t.danger())
                                        .child(format!("−{removed}")),
                                )
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .text_role(&t, TextRole::Xs)
                                        .text_color(t.fg_subtlest())
                                        .child("n / p to step"),
                                )
                                .into_any_element(),
                        )
                        .chain(changes.iter().enumerate().map(|(i, c)| {
                            div()
                                .pb(px(space::S3))
                                .when(cursor == Some(i), |el| {
                                    el.child(
                                        div()
                                            .rounded(px(radius::R_CARD + space::HALF))
                                            .border_2()
                                            .border_color(t.focus())
                                            .child(file_card(&t, c, i)),
                                    )
                                })
                                .when(cursor != Some(i), |el| el.child(file_card(&t, c, i)))
                                .into_any_element()
                        }))
                        .collect(),
                    );
                    div().into_any_element()
                }
            }
            Some(SideTab::Diff { title, patch }) => {
                let d = tools::diff_preview(&patch, 0);
                file_card(
                    &t,
                    &FileChange {
                        path: title,
                        added: d.added,
                        removed: d.removed,
                        patches: vec![patch],
                    },
                    0,
                )
            }
            Some(SideTab::Output { title, text }) => div()
                .flex()
                .flex_col()
                .gap(px(space::S2))
                .child(
                    div()
                        .mono_text(&t, TextRole::Sm)
                        .text_color(t.fg_subtle())
                        .child(SharedString::from(format!("$ {title}"))),
                )
                .child(if text.trim().is_empty() {
                    crate::components::empty::empty_compact(
                        &t,
                        IconName::SquareTerminal,
                        "No output yet",
                    )
                } else {
                    log_view::log_lines(&t, &tools::tail(&text, log_view::MAX_LINES), "side-output")
                })
                .into_any_element(),
            Some(SideTab::Plan) => {
                let plan = self
                    .interaction
                    .as_ref()
                    .and_then(|v| v.read(cx).plan_markdown())
                    .or_else(|| {
                        self.model
                            .transcript
                            .rows()
                            .iter()
                            .rev()
                            .find_map(|r| match &r.kind {
                                RowKind::Assistant(a)
                                    if timeline::plan_steps(&a.text).len() >= 2 =>
                                {
                                    Some(a.text.clone())
                                }
                                _ => None,
                            })
                    });
                match plan {
                    Some(md) => div()
                        .text_role(&t, TextRole::Base)
                        .child(TextView::markdown("side-plan", md).selectable(true))
                        .into_any_element(),
                    None => empty_state(
                        &t,
                        IconName::ListChecks,
                        "No plan",
                        Some("Plans from plan mode appear here.".into()),
                        Vec::new(),
                    )
                    .into_any_element(),
                }
            }
            Some(SideTab::Agent { call_id, .. }) => self.render_agent_tab(&call_id, &t, cx),
            Some(SideTab::Tree) => self.render_tree(&t, cx),
            Some(SideTab::Files) => self.render_files(&t, cx),
            Some(SideTab::Panel(id)) => self.render_panel_body(&id, true, window, cx),
            Some(SideTab::Browser) => self.render_browser(window, cx),
        };
        let toolbar = match self.side.current() {
            Some(SideTab::Panel(id)) => self.panel_toolbar(id, &t, cx),
            _ => None,
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(t.window())
            .child(header)
            .children(strip)
            .children(toolbar)
            .child(
                div()
                    .id("side-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.side_scroll)
                    .track_focus(&self.side_focus)
                    .on_key_down(cx.listener(Self::on_pane_key))
                    .on_mouse_down(
                        gpui_kit::MouseButton::Left,
                        cx.listener(|this, _, window, cx| window.focus(&this.side_focus, cx)),
                    )
                    .p(px(space::S3))
                    .map(|el| match review {
                        Some(children) => el.children(children),
                        None => el.child(body),
                    }),
            )
            .into_any_element()
    }

    fn render_launcher(&self, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        let items = self.launch_items(cx);
        div()
            .flex()
            .flex_col()
            .gap(px(space::S3))
            .child(crate::components::section_label::label(t, "Open a tab"))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(space::S2))
                    .children(
                        items
                            .into_iter()
                            .enumerate()
                            .map(|(i, (glyph, label, item))| {
                                div()
                                    .id(ElementId::NamedInteger("launch".into(), i as u64))
                                    .flex()
                                    .items_center()
                                    .gap(px(space::S2_5))
                                    .w(px(size::SIDEBAR * 0.8))
                                    .h(px(size::BUTTON))
                                    .px(px(space::S3))
                                    .rounded(px(radius::R_CARD))
                                    .bg(t.card())
                                    .border_1()
                                    .border_color(t.card_border())
                                    .cursor_pointer()
                                    .hover(|s| s.bg(t.hover()))
                                    .on_click(cx.listener(
                                        move |this, _: &ClickEvent, window, cx| {
                                            this.launch(item.clone(), window, cx);
                                        },
                                    ))
                                    .child(icon(glyph, size::ICON, t.fg_subtle()))
                                    .child(
                                        div()
                                            .truncate()
                                            .text_role(t, TextRole::Base)
                                            .weight_medium()
                                            .child(SharedString::from(label)),
                                    )
                            }),
                    ),
            )
            .into_any_element()
    }

    /// The live child agent behind an Agent tab. A subagent / sidekick tool row resolves
    /// through the row (its `child` id, else the nth spawned agent); a child session the feed
    /// knows directly — `btw`, a dream run — has no tool row and is looked up by agent id.
    fn agent_live(&self, key: &str) -> Option<AgentView<'_>> {
        let rows = self.model.transcript.rows();
        let live = match rows
            .iter()
            .position(|r| matches!(&r.kind, RowKind::Tool(t) if t.call_id == key))
        {
            Some(ix) => {
                let RowKind::Tool(tool) = &rows[ix].kind else {
                    return None;
                };
                let agent = timeline::agent_line(tool)?;
                let nth = rows[..ix]
                    .iter()
                    .filter(|r| {
                        matches!(&r.kind, RowKind::Tool(t) if timeline::agent_line(t).is_some())
                    })
                    .count();
                self.agents.for_row(agent.child.as_deref(), nth)
            }
            None => self.agents.get(key),
        }?;
        Some(AgentView {
            id: &live.info.id,
            owner: &live.info.owner,
            state: live.info.state,
            elapsed_ms: u64::try_from(live.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            steps: live.steps(),
            rows: live.transcript.rows(),
        })
    }

    /// The Agent tab (A2.20 P3, GA-15): a read-only live child timeline from the
    /// `_oino/agents` feed (user prompt bubble, prose, one fold line per tool call), with the
    /// agent state, elapsed time, steps and Stop agent.
    #[allow(clippy::too_many_lines, reason = "header + timeline rows")]
    fn render_agent_tab(&self, call_id: &str, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        let row = self.model.transcript.get(call_id);
        let tool = match row.map(|r| &r.kind) {
            Some(RowKind::Tool(tool)) => Some(tool.as_ref()),
            _ => None,
        };
        let live = self.agent_live(call_id);
        if tool.is_none() && live.is_none() {
            return empty_state(t, IconName::Bot, "Agent finished", None, Vec::new())
                .into_any_element();
        }
        let running_row = tool
            .is_some_and(|tool| matches!(tool.status, ToolStatus::Running | ToolStatus::Preparing));
        let (running, word) = live.as_ref().map_or(
            (running_row, if running_row { "running" } else { "done" }),
            |a| agent_state_word(a.state),
        );
        let running = running || running_row;
        let word = if running { "running" } else { word };
        let elapsed = if running {
            live.as_ref().map(|a| a.elapsed_ms)
        } else {
            tool.and_then(|t| t.duration_ms)
        };
        let steps = live.as_ref().map_or(0, |a| a.steps);
        let task = tool.and_then(timeline::agent_line).map(|a| a.task);
        let mut meta: Vec<String> = Vec::new();
        if steps > 0 {
            meta.push(format!("{steps} step{}", if steps == 1 { "" } else { "s" }));
        }
        if let Some(ms) = elapsed.filter(|ms| *ms >= 1000) {
            meta.push(timeline::duration(ms));
        }
        let stop = live.as_ref().filter(|_| running).map(|a| {
            let id = a.id.to_owned();
            Btn::new("agent-stop")
                .label("Stop agent")
                .icon(IconName::Square)
                .size(BtnSize::Sm)
                .variant(Variant::GhostDanger)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let backend = Arc::clone(&this.backend);
                    let id = id.clone();
                    Tokio::spawn(cx, async move { backend.agent_abort(&id).await }).detach();
                }))
        });
        let head = div()
            .flex()
            .items_center()
            .gap(px(space::S2))
            .child(crate::components::chip::status(
                t,
                if running { t.working() } else { t.success() },
                word,
            ))
            .when(!meta.is_empty(), |el| {
                el.child(
                    div()
                        .mono_text(t, TextRole::Xs)
                        .text_color(t.fg_subtlest())
                        .child(meta.join(" · ")),
                )
            })
            .child(div().flex_1())
            .children(stop);
        let summary = format!(
            "Agent session · {word}{}{}",
            if steps > 0 {
                format!(" · {steps} steps")
            } else {
                String::new()
            },
            elapsed
                .filter(|ms| *ms >= 1000)
                .map(|ms| format!(" · {}", timeline::duration(ms)))
                .unwrap_or_default()
        );
        let mut col = div().flex().flex_col().gap(px(space::S2_5)).child(head);
        let has_rows = live.as_ref().is_some_and(|a| !a.rows.is_empty());
        if let Some(task) = task.filter(|s| !has_rows && !s.trim().is_empty()) {
            col = col.child(
                div()
                    .px(px(space::S3))
                    .py(px(space::S2))
                    .rounded(px(radius::R_HARNESS))
                    .bg(t.card())
                    .border_1()
                    .border_color(t.border())
                    .text_role(t, TextRole::Caption)
                    .text_color(t.fg_subtle())
                    .child(SharedString::from(timeline::excerpt(&task, 4, 400))),
            );
        }
        match live {
            Some(a) if !a.rows.is_empty() => {
                let rows = a.rows;
                let start = rows.len().saturating_sub(AGENT_ROWS);
                for (i, r) in rows[start..].iter().enumerate() {
                    col = col.child(child_row(t, a.id, start + i, r));
                }
            }
            _ => {
                let output = tool.map(|t| t.output.clone()).unwrap_or_default();
                if output.trim().is_empty() {
                    col = col.child(crate::components::empty::empty_compact(
                        t,
                        IconName::Bot,
                        if running {
                            "Waiting for the agent's first step…"
                        } else {
                            "No live steps were recorded"
                        },
                    ));
                } else {
                    col = col.child(
                        div().text_role(t, TextRole::Prose).child(
                            TextView::markdown(
                                ElementId::Name(format!("agent-out-{call_id}").into()),
                                output,
                            )
                            .selectable(true),
                        ),
                    );
                }
            }
        }
        col.child(
            div()
                .text_role(t, TextRole::Xs)
                .text_color(t.fg_subtlest())
                .child("Read-only view of the agent's session."),
        )
        // Read-back for the driver: state, steps and elapsed of the child session.
        .automation(crate::automation::Meta::labelled(
            crate::automation::Role::Row,
            summary,
        ))
        .into_any_element()
    }

    fn render_tree(&self, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        let Some(tree) = &self.tree else {
            return empty_state(
                t,
                IconName::ListTree,
                "Loading the session tree…",
                None,
                Vec::new(),
            )
            .into_any_element();
        };
        let rows = tree.rows(0, tree.len());
        div()
            .flex()
            .flex_col()
            .children(rows.into_iter().enumerate().map(|(i, r)| {
                let entry = r.entry_id.clone();
                div()
                    .id(ElementId::NamedInteger("tree".into(), i as u64))
                    .flex()
                    .items_center()
                    .h(px(size::CONTROL_SM))
                    .px(px(space::S1))
                    .rounded(px(radius::R_MD))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover()))
                    .when(r.is_leaf, |el| el.bg(t.selected()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.navigate_tree(&entry, cx);
                    }))
                    .child(
                        div()
                            .mono_text(t, TextRole::Sm)
                            .text_color(t.fg_subtlest())
                            .child(SharedString::from(r.prefix.clone())),
                    )
                    .when_some(r.label.clone(), |el, l| el.child(badge(t, Tone::Info, l)))
                    .child(
                        div()
                            .truncate()
                            .text_role(t, TextRole::Sm)
                            .text_color(if r.on_active_path {
                                t.fg()
                            } else {
                                t.fg_subtle()
                            })
                            .child(SharedString::from(format!(
                                "{}{}",
                                r.text.prefix, r.text.body
                            ))),
                    )
            }))
            .into_any_element()
    }

    fn render_files(&self, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        if let Some((path, text)) = &self.files.open {
            let rel = path
                .strip_prefix(&self.model.cwd)
                .unwrap_or(path)
                .display()
                .to_string();
            let lang = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("text")
                .to_owned();
            let mention = format!("@{rel}");
            return div()
                .flex()
                .flex_col()
                .gap(px(space::S2))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::S2))
                        .child(
                            Btn::icon_only("files-back", IconName::ArrowLeft, "Back")
                                .size(BtnSize::Sm)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.files.open = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .mono_text(t, TextRole::Sm)
                                .child(SharedString::from(rel.clone())),
                        )
                        .child(
                            Btn::new("files-mention")
                                .label("Add to prompt")
                                .icon(IconName::Plus)
                                .size(BtnSize::Sm)
                                .variant(Variant::Outline)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.pending_tokens.push((
                                        mention.clone(),
                                        mention.clone(),
                                        rel.clone(),
                                    ));
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    TextView::markdown("file-view", format!("```{lang}\n{text}\n```"))
                        .selectable(true),
                )
                .into_any_element();
        }
        let rows = self.files.rows();
        if rows.is_empty() {
            return empty_state(
                t,
                IconName::FolderOpen,
                "No files",
                Some(SharedString::from(format!(
                    "{} is empty or cannot be read.",
                    self.model.cwd
                ))),
                Vec::new(),
            )
            .into_any_element();
        }
        div()
            .flex()
            .flex_col()
            .children(
                rows.into_iter()
                    .enumerate()
                    .map(|(i, (path, is_dir, depth))| {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let open = is_dir && self.files.expanded.contains(&path);
                        #[allow(clippy::cast_precision_loss, reason = "depth ≤ 8")]
                        let indent = depth as f32 * space::S4;
                        div()
                            .id(ElementId::NamedInteger("file".into(), i as u64))
                            .flex()
                            .items_center()
                            .gap(px(space::S1_5))
                            .h(px(size::CONTROL_SM))
                            .pl(px(indent + space::S1))
                            .rounded(px(radius::R_MD))
                            .cursor_pointer()
                            .hover(|s| s.bg(t.hover()))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.files.toggle(path.clone(), is_dir);
                                cx.notify();
                            }))
                            .child(icon(
                                if is_dir {
                                    if open {
                                        IconName::FolderOpen
                                    } else {
                                        IconName::Folder
                                    }
                                } else {
                                    IconName::File
                                },
                                size::ICON_MD,
                                t.fg_subtle(),
                            ))
                            .child(
                                div()
                                    .truncate()
                                    .text_role(t, TextRole::Sm)
                                    .child(SharedString::from(name)),
                            )
                    }),
            )
            .into_any_element()
    }

    /// Toolbar row of a panel (its `keys` as ghost buttons with `Kbd`).
    pub(crate) fn panel_toolbar(
        &self,
        id: &str,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let panel = self.model.ui.panel(id)?;
        if panel.keys.is_empty() {
            return None;
        }
        let on_action = self.action_fn(id, cx);
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(space::S1))
                .h(px(size::CONTROL_LG))
                .px(px(space::S2))
                .border_b_1()
                .border_color(t.border())
                .children(panel.keys.iter().map(|k| {
                    let a = Rc::clone(&on_action);
                    let action = k.action_id.clone();
                    Btn::new(ElementId::Name(format!("{id}-key-{}", k.action_id).into()))
                        .label(SharedString::from(k.label.clone()))
                        .kbd(SharedString::from(k.key.clone()))
                        .size(BtnSize::Sm)
                        .on_click(move |_, _, cx| a(&action, JsonValue::Null, cx))
                }))
                .into_any_element(),
        )
    }

    /// A panel's body (kanboard boards render natively).
    pub(crate) fn render_panel_body(
        &mut self,
        id: &str,
        roomy: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let Some(panel) = self.model.ui.panel(id).cloned() else {
            return div().into_any_element();
        };
        let on_action = self.action_fn(id, cx);
        let mut o = Ous {
            t,
            on_action,
            inputs: Some(&mut self.inputs),
            cols: crate::components::ous_cols(self.width),
            roomy,
        };
        if panel.id == kanban::BOARD_PANEL
            && let Some(board) = kanban::recognise(&panel.spec)
        {
            let opts = ous::kanban::BoardOptions {
                inline_detail: true,
                ..Default::default()
            };
            return ous::kanban::board(&mut o, &board, &opts, id, window, cx);
        }
        ous::render(&panel.spec, id, &mut o, window, cx)
    }

    fn terminal_title(&self, id: u64) -> String {
        #[cfg(feature = "terminal")]
        if let Some(tab) = self.terminals.iter().find(|t| t.id == id) {
            let n = self.terminals.iter().position(|t| t.id == id).unwrap_or(0) + 1;
            return format!(
                "Terminal {n}{}",
                if tab.views.len() > 1 { " ⫼" } else { "" }
            );
        }
        format!("Terminal {id}")
    }

    /// Trailing buttons of the bottom tab bar: new terminal, split.
    fn bottom_actions(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        #[cfg(feature = "terminal")]
        {
            let on_terminal = matches!(
                self.bottom.0.get(self.bottom.1),
                Some(BottomTab::Terminal(_))
            );
            let mut out = vec![
                Btn::icon_only("term-new", IconName::Plus, "New terminal")
                    .tooltip("New terminal", Some("⌃⇧`"))
                    .size(BtnSize::Sm)
                    .on_click(cx.listener(|this, _, window, cx| this.new_terminal(window, cx)))
                    .into_any_element(),
            ];
            if on_terminal {
                out.push(
                    Btn::icon_only("term-split", IconName::Columns2, "Split terminal")
                        .size(BtnSize::Sm)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.split_terminal(window, cx)),
                        )
                        .into_any_element(),
                );
            }
            out
        }
        #[cfg(not(feature = "terminal"))]
        {
            let _ = cx;
            Vec::new()
        }
    }

    fn render_terminal_tab(&self, id: u64, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        #[cfg(feature = "terminal")]
        if let Some(tab) = self.terminals.iter().find(|tab| tab.id == id) {
            let n = tab.views.len();
            return div()
                .flex()
                .flex_row()
                .flex_1()
                .min_h_0()
                .gap(px(space::S1))
                .px(px(space::S2))
                .py(px(space::S1))
                .children(tab.views.iter().enumerate().map(|(i, v)| {
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .when(n > 1 && i > 0, |el| {
                            el.border_l_1().border_color(t.border()).pl(px(space::S1))
                        })
                        .child(v.clone())
                }))
                .into_any_element();
        }
        let _ = (id, cx);
        empty_state(
            t,
            IconName::SquareTerminal,
            "Terminal closed",
            None,
            Vec::new(),
        )
        .into_any_element()
    }

    /// Opens a task's log (the bounded tail of its output file, read off the UI thread) as an
    /// Output side tab.
    fn open_task_log(&mut self, task: &BgTask, cx: &mut Context<Self>) {
        let path = PathBuf::from(&task.output_path);
        let title = task.name.clone();
        let fallback = task.tail.join("\n");
        let read = gpui_kit::AppContext::background_spawn(cx, async move {
            read_tail(&path, TASK_LOG_BYTES, log_view::MAX_LINES)
        });
        cx.spawn(async move |this, cx| {
            let text = match read.await {
                Ok(lines) if !lines.is_empty() => lines.join("\n"),
                Err(e) if fallback.is_empty() => format!("Cannot read the log: {e}"),
                Ok(_) | Err(_) => fallback,
            };
            let _ = this.update(cx, |p, cx| p.open_side(SideTab::Output { title, text }, cx));
        })
        .detach();
    }

    /// Stops a running task through background-tasks' tray action (`stop:<id>`).
    pub(crate) fn stop_task(&mut self, id: &str, cx: &mut Context<Self>) {
        let backend = Arc::clone(&self.backend);
        let payload = serde_json::json!({"tab": "bg", "actionId": format!("stop:{id}")});
        Tokio::spawn(cx, async move {
            backend
                .service_call(Some("unipi.background-tasks"), "work-tray.action", payload)
                .await
        })
        .detach();
    }

    /// The Tasks tab (A2.20 P3, GA-14): rows from the `unipi:bg:tasks` topic — name, command
    /// (mono 12), status capsule, elapsed, Logs / Stop pills.
    fn render_tasks(&self, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        let tasks = self
            .model
            .topic(BG_TASKS_TOPIC)
            .map(bg_tasks)
            .unwrap_or_default();
        if tasks.is_empty() {
            return empty_state(
                t,
                IconName::Terminal,
                "No background tasks",
                Some(
                    "Start one with /bg <command>. Tasks the agent starts show up here too.".into(),
                ),
                Vec::new(),
            )
            .into_any_element();
        }
        let now = now_ms();
        div()
            .flex()
            .flex_col()
            .rounded(px(radius::R_CARD))
            .bg(t.card())
            .border_1()
            .border_color(t.card_border())
            .overflow_hidden()
            .children(tasks.into_iter().enumerate().map(|(i, task)| {
                let (color, word) = match task.status.as_str() {
                    "running" => (t.working(), "running".to_owned()),
                    "completed" => (t.success(), "done".to_owned()),
                    "killed" => (t.fg_subtle(), "stopped".to_owned()),
                    _ => (
                        t.danger(),
                        task.exit_code
                            .map_or_else(|| "failed".to_owned(), |c| format!("exit {c}")),
                    ),
                };
                let log_task = task.clone();
                let stop_id = task.id.clone();
                let running = task.running();
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S3))
                    .min_h(px(size::ROW_COMPACT))
                    .px(px(space::S3_5))
                    .when(i > 0, |el| el.border_t_1().border_color(t.row_divider()))
                    .child(icon(
                        IconName::Terminal,
                        size::ICON,
                        if running { t.working() } else { t.fg_subtle() },
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_role(t, TextRole::Base)
                                    .weight_medium()
                                    .text_color(t.fg())
                                    .child(SharedString::from(task.name.clone())),
                            )
                            .when(task.command != task.name, |el| {
                                el.child(
                                    div()
                                        .truncate()
                                        .mono_text(t, TextRole::Sm)
                                        .text_color(t.fg_subtlest())
                                        .child(SharedString::from(task.command.clone())),
                                )
                            }),
                    )
                    .child(crate::components::chip::status(t, color, word))
                    .child(
                        div()
                            .w(px(size::BUTTON))
                            .flex()
                            .justify_end()
                            .mono_text(t, TextRole::Xs)
                            .text_color(t.fg_subtlest())
                            .child(timeline::duration(task.elapsed_ms(now))),
                    )
                    .child(
                        Btn::new(ElementId::NamedInteger("task-logs".into(), i as u64))
                            .label("Logs")
                            .size(BtnSize::Sm)
                            .variant(Variant::Outline)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_task_log(&log_task, cx);
                            })),
                    )
                    .when(running, |el| {
                        el.child(
                            Btn::new(ElementId::NamedInteger("task-stop".into(), i as u64))
                                .label("Stop")
                                .size(BtnSize::Sm)
                                .variant(Variant::GhostDanger)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.stop_task(&stop_id, cx);
                                })),
                        )
                    })
            }))
            .into_any_element()
    }

    /// The bottom frame: Terminal tabs (desktop), Output (shell commands of this session),
    /// Tasks, `bottomPane` panels.
    pub fn render_bottom(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = OinoTheme::get(cx);
        if self.bottom.0.is_empty() {
            self.bottom.0 = vec![BottomTab::Output, BottomTab::Tasks];
            // The bottom frame opens on a shell (design.md A1.5).
            #[cfg(feature = "terminal")]
            if self.terminals.is_empty() {
                self.new_terminal(window, cx);
            }
        }
        let running_tasks = self
            .model
            .topic(BG_TASKS_TOPIC)
            .map_or(0, |d| bg_tasks(d).iter().filter(|t| t.running()).count());
        let specs: Vec<TabSpec> = self
            .bottom
            .0
            .iter()
            .map(|b| match b {
                BottomTab::Output => TabSpec {
                    key: "output".into(),
                    icon: IconName::SquareTerminal,
                    title: "Output".into(),
                    closable: false,
                },
                BottomTab::Tasks => TabSpec {
                    key: "tasks".into(),
                    icon: IconName::Terminal,
                    title: match running_tasks {
                        0 => "Tasks".into(),
                        n => SharedString::from(format!("Tasks · {n}")),
                    },
                    closable: false,
                },
                BottomTab::Terminal(id) => TabSpec {
                    key: SharedString::from(format!("term-{id}")),
                    icon: IconName::SquareTerminal,
                    title: SharedString::from(self.terminal_title(*id)),
                    closable: true,
                },
                BottomTab::Panel(id) => TabSpec {
                    key: SharedString::from(id.clone()),
                    icon: IconName::Blocks,
                    title: SharedString::from(
                        self.model
                            .ui
                            .panel(id)
                            .map(|p| p.title.clone())
                            .unwrap_or_default(),
                    ),
                    closable: true,
                },
            })
            .collect();
        let entity = cx.entity().downgrade();
        let e2 = entity.clone();
        let bar = tab_strip(
            "bottom",
            &t,
            StripSize::Compact,
            &specs,
            self.bottom.1,
            Rc::new(move |i, _, cx| {
                let _ = entity.update(cx, |p, cx| {
                    p.bottom.1 = i;
                    cx.notify();
                });
            }),
            Rc::new(move |i, _, cx| {
                let _ = e2.update(cx, |p, cx| {
                    match p.bottom.0.get(i).cloned() {
                        Some(BottomTab::Panel(id)) => p.close_panel(&id, cx),
                        #[cfg(feature = "terminal")]
                        Some(BottomTab::Terminal(id)) => p.close_terminal(id, cx),
                        _ => {}
                    }
                    p.bottom.1 = p.bottom.1.min(p.bottom.0.len().saturating_sub(1));
                    cx.notify();
                });
            }),
            self.bottom_actions(cx),
        );
        let body = match self.bottom.0.get(self.bottom.1).cloned() {
            Some(BottomTab::Terminal(id)) => {
                return div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .child(bar)
                    .child(self.render_terminal_tab(id, &t, cx))
                    .into_any_element();
            }
            Some(BottomTab::Output) | None => {
                let rows = self.model.transcript.rows();
                let mut lines: Vec<String> = Vec::new();
                for r in rows {
                    match &r.kind {
                        RowKind::Bash(b) => {
                            lines.push(format!("$ {}", b.command));
                            lines.extend(tools::tail(&b.output, 200));
                        }
                        RowKind::Tool(tool) if ToolKind::of(&tool.name) == ToolKind::Shell => {
                            lines.push(format!(
                                "$ {}",
                                tool.args
                                    .get("command")
                                    .and_then(JsonValue::as_str)
                                    .unwrap_or_default()
                            ));
                            lines.extend(tools::tail(&tool.output, 200));
                        }
                        _ => {}
                    }
                }
                if lines.is_empty() {
                    empty_state(
                        &t,
                        IconName::SquareTerminal,
                        "No output yet",
                        Some(
                            "Shell commands and bash tool calls of this session appear here."
                                .into(),
                        ),
                        Vec::new(),
                    )
                    .into_any_element()
                } else {
                    let start = lines.len().saturating_sub(5000);
                    log_view::log_lines(&t, &lines[start..], "bottom-output")
                }
            }
            Some(BottomTab::Tasks) => self.render_tasks(&t, cx),
            Some(BottomTab::Panel(id)) => self.render_panel_body(&id, true, window, cx),
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(bar)
            .child(
                div()
                    .id("bottom-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(space::S3))
                    .child(body),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_unified_hunks() {
        let patch = "--- a/x\n+++ b/x\n@@ -3,3 +3,4 @@\n a\n-b\n+c\n+d\n e";
        let (lines, hidden) = number_diff(patch, 100);
        assert_eq!(hidden, 0);
        assert_eq!(lines[0].kind, DiffLineKind::Hunk);
        assert_eq!(
            (lines[1].old, lines[1].new, lines[1].text.as_str()),
            (Some(3), Some(3), "a")
        );
        assert_eq!(
            (lines[2].old, lines[2].new, lines[2].text.as_str()),
            (Some(4), None, "b")
        );
        assert_eq!((lines[3].old, lines[3].new), (None, Some(4)));
        assert_eq!((lines[4].old, lines[4].new), (None, Some(5)));
        assert_eq!((lines[5].old, lines[5].new), (Some(5), Some(6)));
    }

    #[test]
    fn unnumbered_preview_keeps_text() {
        let (lines, _) = number_diff("+new\n ctx", 10);
        assert_eq!(lines[0].new, None);
        assert_eq!(lines[0].text, "new");
        assert_eq!(lines[1].text, " ctx");
    }

    #[test]
    fn parses_bg_tasks_topic() {
        let data = serde_json::json!({"tasks": [
            {"id": "a", "name": "build", "command": "cargo build", "status": "completed",
             "startTime": 1000, "endTime": 4000, "exitCode": 0, "outputPath": "/tmp/a"},
            {"id": "b", "command": "sleep 45", "status": "running", "startTime": 2000,
             "outputTail": ["x"]},
            {"command": "no id"}
        ]});
        let tasks = bg_tasks(&data);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].id, "b");
        assert_eq!(tasks[0].name, "sleep 45");
        assert!(tasks[0].running());
        assert_eq!(tasks[0].tail, vec!["x".to_owned()]);
        assert_eq!(tasks[1].elapsed_ms(9999), 3000);
        assert_eq!(tasks[1].exit_code, Some(0));
        assert!(bg_tasks(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn reads_a_bounded_tail() {
        let dir = std::env::temp_dir().join(format!("oino-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("log");
        let text = (0..1000).fold(String::new(), |mut s, i| {
            use std::fmt::Write as _;
            let _ = writeln!(s, "line {i}");
            s
        });
        std::fs::write(&path, text).unwrap();
        let tail = read_tail(&path, 64, 100).unwrap();
        assert_eq!(tail.last().map(String::as_str), Some("line 999"));
        assert!(tail.len() < 10);
        assert!(tail[0].starts_with("line "));
        assert_eq!(read_tail(&path, 1 << 20, 3).unwrap().len(), 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
