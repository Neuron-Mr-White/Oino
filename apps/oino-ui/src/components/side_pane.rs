//! Side frame (design.md §4.20, D4) and bottom frame (D7) contents: Review (`DiffViewer`),
//! Files (tree + code viewer), Plan, Agent, Tree, extension panels, single diffs/outputs;
//! Output and Tasks bottom tabs.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::text::TextView;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    px,
};
use oino_ai::JsonValue;

use crate::components::badge::badge;
use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::empty::empty_state;
use crate::components::log_view;
use crate::components::ous::{self, Ous, diff_lines};
use crate::components::tabs::{TabSpec, frame_tab_bar};
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::kanban;
use crate::model::ous::Tone;
use crate::model::timeline::{self, FileChange, split_path};
use crate::model::tools::{self, ToolKind};
use crate::model::transcript::{RowKind, ToolStatus};
use crate::views::session_pane::{BottomTab, SessionPane, SideTab};

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

fn tab_spec(tab: &SideTab, title: &str) -> TabSpec {
    let (glyph, label) = match tab {
        SideTab::Review => (IconName::ListChecks, "Review".to_owned()),
        SideTab::Files => (IconName::FolderOpen, "Files".to_owned()),
        SideTab::Plan => (IconName::ListChecks, "Plan".to_owned()),
        SideTab::Agent { title, .. } => (IconName::Bot, title.clone()),
        SideTab::Tree => (IconName::ListTree, "Tree".to_owned()),
        SideTab::Panel(id) => (
            if id.starts_with("unipi.kanboard") {
                IconName::Kanban
            } else if id.starts_with("unipi.app-bridge") {
                IconName::Smartphone
            } else {
                IconName::Blocks
            },
            title.to_owned(),
        ),
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

/// Per-file card of the `DiffViewer`.
fn file_card(t: &OinoTheme, change: &FileChange, key: usize) -> AnyElement {
    let (name, dir) = split_path(&change.path);
    let patch = change.patches.join("\n");
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
                .px(px(space::S3))
                .h(px(size::CONTROL_LG))
                .border_b_1()
                .border_color(t.border())
                .child(
                    div()
                        .mono_text(t, TextRole::Sm)
                        .weight_medium()
                        .text_color(t.git_modified())
                        .child("M"),
                )
                .child(
                    div()
                        .mono_text(t, TextRole::Sm)
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
                        .mono_text(t, TextRole::Sm)
                        .text_color(t.diff_added())
                        .child(format!("+{}", change.added)),
                )
                .child(
                    div()
                        .mono_text(t, TextRole::Sm)
                        .text_color(t.diff_removed())
                        .child(format!("−{}", change.removed)),
                ),
        )
        .child(
            div()
                .id(ElementId::NamedInteger("review-file".into(), key as u64))
                .overflow_x_scroll()
                .child(diff_lines(t, &patch, 2000)),
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
            cx.emit(crate::views::session_pane::PaneEvent::Toast(
                "Could not start a shell".into(),
            ));
            return;
        };
        view.update(cx, |v, cx| v.focus(window, cx));
        let id = self.next_terminal;
        self.next_terminal += 1;
        self.terminals
            .push(crate::views::session_pane::TerminalTab::new(id, view, sub));
        self.bottom.0.push(BottomTab::Terminal(id));
        self.bottom.1 = self.bottom.0.len() - 1;
        cx.emit(crate::views::session_pane::PaneEvent::ShowBottom);
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
        cx.emit(crate::views::session_pane::PaneEvent::ShowBottom);
        cx.notify();
    }

    fn side_tab_bar(&self, t: &OinoTheme, specs: &[TabSpec], cx: &mut Context<Self>) -> AnyElement {
        let entity = cx.entity().downgrade();
        let e2 = entity.clone();
        let on_select: Rc<dyn Fn(usize, &mut Window, &mut App)> = Rc::new(move |i, _, cx| {
            let _ = entity.update(cx, |p, cx| {
                p.side.active = i;
                cx.notify();
            });
        });
        let on_close: Rc<dyn Fn(usize, &mut Window, &mut App)> = Rc::new(move |i, _, cx| {
            let _ = e2.update(cx, |p, cx| {
                match p.side.tabs.get(i).cloned() {
                    Some(SideTab::Panel(id)) => p.close_panel(&id, cx),
                    #[cfg(feature = "browser")]
                    Some(SideTab::Browser) => {
                        p.browser = None;
                        p.side.close_at(i);
                    }
                    _ => {
                        p.side.close_at(i);
                    }
                }
                cx.notify();
            });
        });
        let launcher = Btn::icon_only("side-add", IconName::Plus, "Open a tab")
            .size(BtnSize::Sm)
            .on_click(cx.listener(|this, _, _, cx| {
                this.side.tabs.retain(|t| *t != SideTab::Review);
                this.side.open(SideTab::Review);
                cx.notify();
            }))
            .into_any_element();
        let mut trailing = vec![launcher];
        if self.header_state.controls_side {
            trailing.push(crate::components::window_chrome::window_controls(
                t,
                self.header_state.maximized,
            ));
        }
        frame_tab_bar(
            "side",
            t,
            specs,
            self.side.active,
            on_select,
            on_close,
            trailing,
        )
        .into_any_element()
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
                        crate::components::browser::BrowserView::new(&url, window, cx)
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

    /// The side frame: tab bar + active tab content (launcher grid when empty).
    #[allow(clippy::too_many_lines, reason = "one arm per tab kind")]
    pub fn render_side(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = OinoTheme::get(cx);
        if self.side.current() == Some(&SideTab::Browser) {
            // The browser fills the pane edge to edge (no padding / scroll wrapper).
            let specs: Vec<TabSpec> = self
                .side
                .tabs
                .iter()
                .map(|tab| {
                    let title = match tab {
                        SideTab::Browser => self.browser_title(cx),
                        SideTab::Panel(id) => self
                            .model
                            .ui
                            .panel(id)
                            .map(|p| p.title.clone())
                            .unwrap_or_default(),
                        _ => String::new(),
                    };
                    tab_spec(tab, &title)
                })
                .collect();
            let bar = self.side_tab_bar(&t, &specs, cx);
            let body = self.render_browser(window, cx);
            return div()
                .flex()
                .flex_col()
                .size_full()
                .child(bar)
                .child(div().flex_1().min_h_0().child(body))
                .into_any_element();
        }
        // Panels closed by their extension drop their tabs.
        let ui = &self.model.ui;
        self.side.tabs.retain(|tab| match tab {
            SideTab::Panel(id) => ui.panel(id).is_some(),
            _ => true,
        });
        if self.side.active >= self.side.tabs.len() {
            self.side.active = self.side.tabs.len().saturating_sub(1);
        }
        let specs: Vec<TabSpec> = self
            .side
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
            .collect();
        let bar = self.side_tab_bar(&t, &specs, cx);
        let mut review: Option<Vec<AnyElement>> = None;
        let body: AnyElement =
            match self.side.current().cloned() {
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
                                    .pb(px(space::S3))
                                    .text_role(&t, TextRole::Sm)
                                    .text_color(t.fg_subtle())
                                    .child(format!(
                                        "{} file{} changed · +{added} −{removed} · n / p to step",
                                        changes.len(),
                                        if changes.len() == 1 { "" } else { "s" }
                                    ))
                                    .into_any_element(),
                            )
                            .chain(changes.iter().enumerate().map(|(i, c)| {
                                div()
                                    .pb(px(space::S3))
                                    .when(cursor == Some(i), |el| {
                                        el.child(
                                            div()
                                                .rounded(px(radius::R_CARD))
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
                    .child(log_view::log_lines(
                        &t,
                        &tools::tail(&text, 5000),
                        "side-output",
                    ))
                    .into_any_element(),
                Some(SideTab::Plan) => {
                    let plan =
                        self.interaction
                            .as_ref()
                            .and_then(|v| v.read(cx).plan_markdown())
                            .or_else(|| {
                                self.model.transcript.rows().iter().rev().find_map(|r| {
                                    match &r.kind {
                                        RowKind::Assistant(a)
                                            if timeline::plan_steps(&a.text).len() >= 2 =>
                                        {
                                            Some(a.text.clone())
                                        }
                                        _ => None,
                                    }
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
                Some(SideTab::Agent { call_id, title }) => {
                    self.render_agent_tab(&call_id, &title, &t, cx)
                }
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
            .child(bar)
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
        let mut items: Vec<(IconName, String, SideTab)> = vec![
            (IconName::ListChecks, "Review".into(), SideTab::Review),
            (IconName::FolderOpen, "Files".into(), SideTab::Files),
            (IconName::ListChecks, "Plan".into(), SideTab::Plan),
            (IconName::ListTree, "Tree".into(), SideTab::Tree),
        ];
        if crate::platform::BrowserSupport::get(cx) {
            items.push((IconName::Globe, "Browser".into(), SideTab::Browser));
        }
        for (id, title) in self.side_panels() {
            items.push((IconName::Blocks, title, SideTab::Panel(id)));
        }
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(space::S2))
            .children(
                items
                    .into_iter()
                    .enumerate()
                    .map(|(i, (glyph, label, tab))| {
                        div()
                            .id(ElementId::NamedInteger("launch".into(), i as u64))
                            .flex()
                            .flex_col()
                            .gap(px(space::S2))
                            .w(px(size::SIDEBAR / 2.0))
                            .p(px(space::S3))
                            .rounded(px(radius::R_CARD))
                            .border_1()
                            .border_color(t.border())
                            .cursor_pointer()
                            .hover(|s| s.bg(t.hover()))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.open_side(tab.clone(), cx);
                            }))
                            .child(icon(glyph, size::ICON, t.fg_subtle()))
                            .child(
                                div()
                                    .text_role(t, TextRole::Base)
                                    .child(SharedString::from(label)),
                            )
                    }),
            )
            .into_any_element()
    }

    fn render_agent_tab(
        &self,
        call_id: &str,
        title: &str,
        t: &OinoTheme,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = self.model.transcript.get(call_id);
        let Some(RowKind::Tool(tool)) = row.map(|r| &r.kind) else {
            return empty_state(t, IconName::Bot, "Agent finished", None, Vec::new())
                .into_any_element();
        };
        let agent = timeline::agent_line(tool);
        let running = matches!(tool.status, ToolStatus::Running | ToolStatus::Preparing);
        div()
            .flex()
            .flex_col()
            .gap(px(space::S3))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S2))
                    .child(icon(IconName::Bot, size::ICON, t.agent_node_fg()))
                    .child(div().text_role(t, TextRole::Base).weight_medium().child(SharedString::from(title.to_owned())))
                    .child(badge(t, if running { Tone::Warning } else { Tone::Success }, if running { "running" } else { "done" }))
                    .when_some(tool.duration_ms, |el, ms| el.child(div().mono_text(t, TextRole::Sm).text_color(t.fg_subtle()).child(timeline::duration(ms)))),
            )
            .when_some(agent.map(|a| a.task), |el, task| {
                el.child(div().p(px(space::S3)).rounded(px(radius::R_LG)).bg(t.surface()).text_role(t, TextRole::Base).child(SharedString::from(task)))
            })
            .child(
                div()
                    .text_role(t, TextRole::Base)
                    .child(TextView::markdown("agent-out", if tool.output.is_empty() { "_Waiting for the agent…_".to_owned() } else { tool.output.clone() }).selectable(true)),
            )
            .child(div().text_role(t, TextRole::Xs).text_color(t.fg_subtlest()).child("Read-only. Live child timelines need the `_oino/agents` feed (design.md R5)."))
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
                    title: "Tasks".into(),
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
        let bar = frame_tab_bar(
            "bottom",
            &t,
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
            Some(BottomTab::Tasks) => {
                let tasks: Vec<_> = self
                    .status
                    .run
                    .iter()
                    .filter(|r| r.kind == crate::model::status::RunKind::Task)
                    .cloned()
                    .collect();
                if tasks.is_empty() {
                    empty_state(
                        &t,
                        IconName::Terminal,
                        "No background tasks",
                        None,
                        Vec::new(),
                    )
                    .into_any_element()
                } else {
                    div()
                        .flex()
                        .flex_col()
                        .children(tasks.into_iter().map(|task| {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(space::S3))
                                .h(px(size::CONTROL_LG))
                                .px(px(space::S2))
                                .border_b_1()
                                .border_color(t.border())
                                .child(icon(IconName::Terminal, size::ICON_MD, t.fg_subtle()))
                                .child(
                                    div()
                                        .flex_1()
                                        .truncate()
                                        .mono_text(&t, TextRole::Sm)
                                        .child(SharedString::from(task.label)),
                                )
                                .child(crate::components::badge::capsule(
                                    crate::components::badge::Capsule::Running,
                                    cx,
                                ))
                        }))
                        .into_any_element()
                }
            }
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
