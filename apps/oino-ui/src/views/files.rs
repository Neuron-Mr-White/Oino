//! The Files page (round5 #24): a Zed-like files view — lazy directory tree on
//! the left, the gpui-base code editor (syntect highlighting, auto-indent,
//! bracket auto-close, in-editor search) on the right, a Problems strip fed by
//! `rust-analyzer` diagnostics, and a status bar with cursor position, language,
//! dirty state and the LSP status.
//!
//! Scope (round5.md §6): open/edit/save, format-on-save (LSP `formatting`, then
//! a `rustfmt` fallback), diagnostics squiggles + Problems list, hover popover,
//! completions. Deferred: tree-sitter grammars, the dock workbench, multiple
//! buffers/tabs, non-Rust servers, go-to-definition.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::highlighter::Diagnostic;
use gpui_kit::component::input::{
    Editor, EditorState, InputEvent, Position, RopeExt as _, TabSize,
};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::component::tree::{TreeEvent, TreeState, tree};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div,
    px, relative,
};
use tokio::sync::mpsc;

use crate::automation::{Automatable as _, Meta, Role};
use crate::components::button::{Btn, BtnSize, Variant};

use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, space};
use crate::editor::bridge::{self, EditorLsp, LspEvent, LspStatus};
use crate::editor::lsp;
use crate::editor::prefs::EditorPrefs;
use crate::editor::tree::FileTreeModel;

/// Buffers larger than this are rejected before the rope ever sees them.
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// The status/probe `value` caps at this many characters of buffer text.
const PROBE_VALUE_CHARS: usize = 200_000;

/// Layout metrics (px) — named so `no_literal_styles` keeps them out of view code.
const TREE_WIDTH: f32 = 240.0;
const TREE_MIN: f32 = 160.0;
const TREE_MAX: f32 = 420.0;
const TREE_INDENT: f32 = 14.0;
const TREE_PAD: f32 = 6.0;
const PROBLEMS_MAX_H: f32 = 180.0;
const STATUS_H: f32 = 26.0;

/// What the Files page tells the shell (toasts live outside the page).
pub enum FilesEvent {
    /// Show `text` in the toast stack.
    Toast(SharedString),
}

impl EventEmitter<FilesEvent> for FilesView {}

/// The Files page entity. The shell keeps one instance alive across route
/// switches so the open file, the tree expansion and the language server all
/// persist.
pub struct FilesView {
    focus: FocusHandle,
    /// The project dir the tree is rooted at.
    root: PathBuf,
    agent_dir: PathBuf,
    model: FileTreeModel,
    tree: Entity<TreeState>,
    editor: Entity<EditorState>,
    lsp: EditorLsp,
    prefs: EditorPrefs,
    /// The file in the editor.
    open: Option<PathBuf>,
    /// Unsaved changes exist in the buffer.
    dirty: bool,
    /// Unsaved buffers of files the user navigated away from (single-buffer
    /// editor; stashing beats silently discarding edits).
    stashed: HashMap<PathBuf, String>,
    problems_open: bool,
    /// The diagnostics generation last pushed into the squiggle set.
    diag_applied: u64,
    /// The buffer load error being shown instead of the editor.
    load_error: Option<String>,
    _subscriptions: Vec<Subscription>,
    _lsp_pump: Task<()>,
}

impl Focusable for FilesView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// The syntect language token for `path` (the extension doubles as the token:
/// `find_syntax_by_token` resolves names and extensions alike).
fn language_of(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
    {
        "rs" => "rust",
        "toml" => "toml",
        "json" | "jsonc" => "json",
        "md" | "mdx" => "markdown",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "jsx" => "jsx",
        "go" => "go",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" => "cpp",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "rb" => "ruby",
        "sh" | "bash" | "zsh" => "bash",
        "yml" | "yaml" => "yaml",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" => "scss",
        "sql" => "sql",
        "xml" => "xml",
        "dockerfile" => "dockerfile",
        "lua" => "lua",
        "vim" => "vim",
        "zig" => "zig",
        "cs" => "csharp",
        "php" => "php",
        "proto" => "proto",
        "graphql" | "gql" => "graphql",
        "diff" | "patch" => "diff",
        _ => "text",
    }
}

/// LSP `languageId` for `path` (`rust-analyzer` only documents Rust this round).
fn lsp_language_id(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => Some("rust"),
        _ => None,
    }
}

impl FilesView {
    /// Builds the page: tree model + entities, editor with LSP providers, the
    /// diagnostics pump.
    pub fn new(
        root: PathBuf,
        agent_dir: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let prefs = EditorPrefs::load(&agent_dir);
        let model = FileTreeModel::new(&root);
        let tree = cx.new(|cx| TreeState::new(cx));
        let initial_items = model.items();
        tree.update(cx, |state, cx| state.set_items(initial_items, cx));

        let (events, mut events_rx) = mpsc::unbounded_channel::<LspEvent>();
        let lsp = EditorLsp::new(&root, prefs.lsp, events);

        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("text")
                .line_number(true)
                .indent_guides(true)
                .tab_size(TabSize {
                    tab_size: 4,
                    hard_tabs: false,
                })
                .soft_wrap(false)
                .placeholder("Open a file from the tree")
        });
        editor.update(cx, |state, _cx| {
            let lsp = Rc::new(lsp.clone());
            state.lsp_mut().completion_provider = Some(lsp.clone());
            state.lsp_mut().hover_provider = Some(lsp);
        });

        let mut subscriptions = Vec::new();
        // Tree expand/collapse → model + rebuild (lazy loading happens here).
        subscriptions.push(cx.subscribe(&tree, |this, _tree, event: &TreeEvent, cx| {
            match event {
                TreeEvent::Expanded(id) => {
                    if let Some(path) = FileTreeModel::path_of(id.as_str()) {
                        this.model.expand(&path);
                    }
                }
                TreeEvent::Collapsed(id) => {
                    if let Some(path) = FileTreeModel::path_of(id.as_str()) {
                        this.model.collapse(&path);
                    }
                }
            }
            let items = this.model.items();
            this.tree.update(cx, |state, cx| state.set_items(items, cx));
        }));
        // Buffer edits → dirty + debounced `didChange`.
        subscriptions.push(cx.subscribe(&editor, |this, _e, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.dirty = true;
                let text = this.editor.read(cx).text().to_string();
                this.lsp.document_changed(text);
                this.lsp.schedule_sync(cx);
                cx.notify();
            }
        }));

        // LSP events (status/diagnostics) wake the page on the UI thread.
        let lsp_pump = cx.spawn_in(window, async move |this, cx| {
            while let Some(_event) = events_rx.recv().await {
                let _ = this.update_in(cx, |this, _window, cx| {
                    this.push_diagnostics(cx);
                    cx.notify();
                });
            }
        });

        Self {
            focus: cx.focus_handle(),
            root,
            agent_dir,
            model,
            tree,
            editor,
            lsp,
            prefs,
            open: None,
            dirty: false,
            stashed: HashMap::new(),
            problems_open: false,
            diag_applied: 0,
            load_error: None,
            _subscriptions: subscriptions,
            _lsp_pump: lsp_pump,
        }
    }

    /// The file path of the open buffer (for the shell's header).
    #[must_use]
    pub fn open_path(&self) -> Option<&Path> {
        self.open.as_deref()
    }

    /// Opens `path` in the editor: reads it, sets language + LSP document.
    pub fn open_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.open.as_ref() == Some(&path) {
            return;
        }
        // Stash the outgoing buffer when it has unsaved changes.
        if self.dirty
            && let Some(current) = self.open.take()
        {
            let text = self.editor.read(cx).text().to_string();
            self.stashed.insert(current.clone(), text);
            self.lsp.close_document(&current);
        } else if let Some(current) = self.open.take() {
            self.lsp.close_document(&current);
        }

        match self.read_buffer(&path) {
            Ok((text, stashed)) => {
                self.load_error = None;
                let language = language_of(&path);
                self.editor.update(cx, |state, cx| {
                    state.set_highlighter(language, cx);
                    state.set_value(text.clone(), window, cx);
                });
                self.dirty = stashed;
                self.open = Some(path.clone());
                if let Some(lang_id) = lsp_language_id(&path) {
                    self.lsp.ensure_started(&path, cx);
                    self.lsp.open_document(&path, lang_id, &text);
                }
            }
            Err(err) => {
                self.load_error = Some(err);
                self.open = None;
                self.dirty = false;
            }
        }
        self.diag_applied = 0;
        self.push_diagnostics(cx);
        cx.notify();
    }

    /// Reads `path`, preferring a stashed unsaved buffer. Returns `(text,
    /// from_stash)` or a user-facing error.
    fn read_buffer(&mut self, path: &Path) -> Result<(String, bool), String> {
        if let Some(text) = self.stashed.remove(path) {
            return Ok((text, true));
        }
        let meta =
            std::fs::metadata(path).map_err(|e| format!("Cannot stat {}: {e}", path.display()))?;
        if meta.len() > MAX_FILE_BYTES {
            return Err(format!(
                "{} is too large to open ({} MB)",
                path.display(),
                meta.len() / 1_048_576
            ));
        }
        let bytes =
            std::fs::read(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
        if bytes.contains(&0) {
            return Err(format!("{} is a binary file", path.display()));
        }
        Ok((String::from_utf8_lossy(&bytes).into_owned(), false))
    }

    /// `⌘S` / the Save button: format-on-save (LSP → `rustfmt` fallback for
    /// Rust), then write the buffer back.
    pub fn save(&mut self, _: &SaveFile, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.open.clone() else {
            return;
        };
        let is_rust = lsp_language_id(&path).is_some();
        let format = if is_rust {
            self.lsp.format(&path, cx)
        } else {
            Task::ready(Ok(Vec::new()))
        };
        let editor = self.editor.clone();
        cx.spawn_in(window, async move |this, cx| {
            let mut edits = format.await.ok().unwrap_or_default();
            if edits.is_empty() && is_rust && bridge::rustfmt_available() {
                // rustfmt fallback: whole-document replacement. `rustfmt` is a
                // tokio future — run it on the runtime, not the UI executor.
                let (text, end) = editor.update(cx, |state, _| {
                    let text = state.text();
                    (text.to_string(), text.offset_to_position(text.len()))
                });
                let formatted = cx
                    .update(|_window, app| {
                        crate::rt::Tokio::handle(app).spawn(bridge::rustfmt(text.clone()))
                    })
                    .ok();
                let formatted = match formatted {
                    Some(join) => join.await.ok().and_then(Result::ok),
                    None => None,
                };
                if let Some(formatted) = formatted
                    && formatted != text
                {
                    edits = vec![lsp_types::TextEdit {
                        range: lsp_types::Range::new(Position::new(0, 0), end),
                        new_text: formatted,
                    }];
                }
            }
            let _ = this.update_in(cx, |this, window, cx| {
                this.finish_save(path, edits, window, cx);
            });
        })
        .detach();
    }

    fn finish_save(
        &mut self,
        path: PathBuf,
        edits: Vec<lsp_types::TextEdit>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open.as_ref() != Some(&path) {
            return;
        }
        if !edits.is_empty() {
            self.editor.update(cx, |state, cx| {
                state.apply_lsp_edits(&edits, window, cx);
            });
        }
        let text = self.editor.read(cx).text().to_string();
        match std::fs::write(&path, &text) {
            Ok(()) => {
                self.dirty = false;
                self.lsp.document_changed(text);
                self.lsp.schedule_sync(cx);
                if let Some(uri) = lsp::file_uri(&path) {
                    self.lsp.did_save(&uri);
                }
                cx.emit(FilesEvent::Toast("Saved".into()));
            }
            Err(err) => {
                cx.emit(FilesEvent::Toast(format!("Save failed: {err}").into()));
            }
        }
        cx.notify();
    }

    /// Pushes the latest server diagnostics into the editor's squiggle set.
    fn push_diagnostics(&mut self, cx: &mut Context<Self>) {
        let generation = self.lsp.diagnostics_gen();
        if generation == self.diag_applied {
            return;
        }
        self.diag_applied = generation;
        let diags = self
            .open
            .as_deref()
            .map(|p| self.lsp.diagnostics(p))
            .unwrap_or_default();
        // New diagnostics open the strip (they deserve attention); an empty
        // set leaves a manually-toggled state alone.
        if !diags.is_empty() {
            self.problems_open = true;
        }
        self.editor.update(cx, |state, _cx| {
            if let Some(set) = state.diagnostics_mut() {
                set.clear();
                set.extend(diags.into_iter().map(|d| {
                    Diagnostic::new(d.range.start..d.range.end, d.message)
                        .with_severity(d.severity.unwrap_or(lsp_types::DiagnosticSeverity::ERROR))
                        .with_source(d.source.unwrap_or_else(|| "rust-analyzer".into()))
                }));
            }
        });
    }

    /// The Problems rows for the open file.
    fn problems(&self) -> Vec<lsp_types::Diagnostic> {
        self.open
            .as_deref()
            .map(|p| self.lsp.diagnostics(p))
            .unwrap_or_default()
    }

    fn toggle_problems(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.problems_open = !self.problems_open;
        cx.notify();
    }

    fn toggle_lsp(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.prefs.lsp = !self.prefs.lsp;
        self.prefs.save(&self.agent_dir);
        self.lsp.set_enabled(self.prefs.lsp);
        if self.prefs.lsp
            && let Some(path) = self.open.clone()
        {
            // Re-arm on the open file.
            if let Some(lang_id) = lsp_language_id(&path) {
                self.lsp.ensure_started(&path, cx);
                let text = self.editor.read(cx).text().to_string();
                self.lsp.open_document(&path, lang_id, &text);
            }
        }
        cx.notify();
    }

    fn refresh_tree(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.model.refresh();
        let items = self.model.items();
        self.tree.update(cx, |state, cx| state.set_items(items, cx));
        cx.notify();
    }

    fn jump_to(&mut self, position: Position, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |state, cx| {
            state.unfold_at(position, cx);
            // set_cursor_position also focuses the editor.
            state.set_cursor_position(position, window, cx);
        });
    }

    // ── rendering ──────────────────────────────────────────────────────────

    fn render_tree(&self, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity();
        let tree_el = tree(&self.tree, move |ix, entry, _selected, _window, cx| {
            view.update(cx, |this, cx| {
                let item = entry.item();
                let folder = entry.is_folder();
                let path = FileTreeModel::path_of(item.id.as_str());
                let is_open = path.as_ref() == this.open.as_ref();
                let t = OinoTheme::get(cx);
                let icon_name = if !folder {
                    IconName::File
                } else if entry.is_expanded() {
                    IconName::FolderOpen
                } else {
                    IconName::Folder
                };
                let label: SharedString = item.label.clone();
                let stub = path.is_none();
                ListItem::new(ix)
                    .w_full()
                    .rounded(px(radius::R_INPUT))
                    .py_0p5()
                    .px_2()
                    .pl(px(TREE_INDENT) * entry.depth() + px(TREE_PAD))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(space::S2))
                            .overflow_hidden()
                            .child(icon(
                                icon_name,
                                14.,
                                if stub { t.fg_subtlest() } else { t.fg_subtle() },
                            ))
                            .child(
                                div()
                                    .text_role(&t, TextRole::Base)
                                    .text_color(if stub {
                                        t.fg_subtlest()
                                    } else if is_open {
                                        t.accent()
                                    } else {
                                        t.fg()
                                    })
                                    .child(label),
                            )
                            .automation_named(
                                Meta::labelled(Role::Row, item.label.to_string()).selected(is_open),
                                format!("file-{}", item.id),
                            ),
                    )
                    .when(!folder && !stub, |el| {
                        el.on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(path) = path.clone() {
                                this.open_file(path, window, cx);
                            }
                        }))
                    })
            })
        })
        .text_sm()
        .p_1()
        .h_full();

        div()
            .id("files-tree")
            .size_full()
            .bg(t.sidebar())
            .child(tree_el)
            .automation(Meta::labelled(Role::Other, "files-tree"))
            .into_any_element()
    }

    fn render_problems(&self, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        let problems = self.problems();
        let rows: Vec<AnyElement> = problems
            .iter()
            .enumerate()
            .map(|(ix, d)| {
                let pos = d.range.start;
                let severity = match d.severity.unwrap_or(lsp_types::DiagnosticSeverity::ERROR) {
                    lsp_types::DiagnosticSeverity::ERROR => ("error", t.danger_surface_fg()),
                    lsp_types::DiagnosticSeverity::WARNING => ("warning", t.warning_surface_fg()),
                    _ => ("info", t.fg_subtle()),
                };
                let message = d.message.clone();
                let label = format!("{}:{} {}", pos.line + 1, pos.character + 1, message);
                let row_pos = pos;
                div()
                    .id(("problem", ix))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(space::S2))
                    .px(px(space::S3))
                    .py(px(space::S1))
                    .rounded(px(radius::R_INPUT))
                    .cursor_pointer()
                    .hover(|el| el.bg(t.surface_hover()))
                    .child(
                        div()
                            .text_role(t, TextRole::Xs)
                            .text_color(severity.1)
                            .child(SharedString::from(severity.0.to_owned())),
                    )
                    .child(
                        div()
                            .mono_text(t, TextRole::Xs)
                            .text_color(t.fg())
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(SharedString::from(label.clone())),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.jump_to(row_pos, window, cx);
                    }))
                    .automation(Meta::labelled(Role::Row, format!("problem {label}")))
                    .into_any_element()
            })
            .collect();

        div()
            .id("files-problems")
            .w_full()
            .max_h(px(PROBLEMS_MAX_H))
            .overflow_hidden()
            .border_t_1()
            .border_color(t.line_soft())
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(space::S3))
                    .pt(px(space::S2))
                    .pb(px(space::S1))
                    .text_role(t, TextRole::Xs)
                    .text_color(t.fg_subtle())
                    .child(format!("Problems ({})", problems.len())),
            )
            .child(
                div()
                    .id("problems-rows")
                    .flex_1()
                    .overflow_y_scroll()
                    .p_1()
                    .children(rows),
            )
            .into_any_element()
    }

    fn render_status(&self, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        let state = self.editor.read(cx);
        let pos = state.cursor_position();
        let name = self
            .open
            .as_deref()
            .and_then(|p| p.strip_prefix(&self.root).ok().or(Some(p)))
            .map_or_else(
                || "No file".to_owned(),
                |p| p.to_string_lossy().into_owned(),
            );
        let language = state.language_name().to_string();
        let lsp_word = match self.lsp.status() {
            LspStatus::Off(why) => format!("LSP {why}"),
            LspStatus::Starting => "LSP starting…".to_owned(),
            LspStatus::Ready => "LSP rust-analyzer".to_owned(),
            LspStatus::Failed(err) => format!("LSP failed: {err}"),
        };
        let dirty_mark = if self.dirty { " ●" } else { "" };
        let meta = format!(
            "{name}{dirty_mark} · {language} · {}:{} · {lsp_word}",
            pos.line + 1,
            pos.character + 1
        );

        div()
            .id("files-status")
            .w_full()
            .h(px(STATUS_H))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(space::S3))
            .border_t_1()
            .border_color(t.line_soft())
            .child(
                div()
                    .text_role(t, TextRole::Xs)
                    .text_color(if self.dirty { t.fg() } else { t.fg_subtle() })
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(SharedString::from(meta.clone())),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(space::S2))
                    .child(
                        Btn::new("files-lsp-toggle")
                            .label(if self.prefs.lsp { "LSP on" } else { "LSP off" })
                            .icon(IconName::Sparkles)
                            .size(BtnSize::Xs)
                            .on_click(cx.listener(Self::toggle_lsp)),
                    ),
            )
            .automation_named(
                Meta::labelled(Role::Other, meta).value({
                    // The driver's `text` assert reads labels and values: park
                    // the buffer here so a scenario can prove file content
                    // arrived.
                    let text = state.text().to_string();
                    text.chars().take(PROBE_VALUE_CHARS).collect::<String>()
                }),
                "files-status",
            )
            .into_any_element()
    }

    fn render_editor(&self, t: &OinoTheme, _cx: &mut Context<Self>) -> AnyElement {
        if let Some(err) = &self.load_error {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_role(t, TextRole::Base)
                        .text_color(t.fg_subtle())
                        .child(SharedString::from(err.clone())),
                )
                .into_any_element();
        }
        if self.open.is_none() {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_role(t, TextRole::Base)
                        .text_color(t.fg_subtle())
                        .child("Select a file from the tree"),
                )
                .into_any_element();
        }
        Editor::new(&self.editor)
            .bordered(false)
            .appearance(false)
            .p_0()
            .h(relative(1.))
            .w_full()
            .font_family(t.mono_font())
            .text_size(px(t.code))
            .into_any_element()
    }
}

impl Render for FilesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = OinoTheme::get(cx);
        // Diagnostics pushed lazily too (the pump may have run while another
        // route was visible — render catches up before painting).
        self.push_diagnostics(cx);

        let problems = self.problems();
        let problem_count = problems.len();
        let show_problems = self.problems_open && !problems.is_empty();

        let tools = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(space::S2))
            .child(
                Btn::new("files-problems")
                    .label(format!("Problems ({problem_count})"))
                    .icon(IconName::TriangleAlert)
                    .size(BtnSize::Sm)
                    .when(self.problems_open, |b| b.variant(Variant::Primary))
                    .on_click(cx.listener(Self::toggle_problems)),
            )
            .child(
                Btn::new("files-refresh")
                    .label("Refresh")
                    .icon(IconName::RefreshCw)
                    .size(BtnSize::Sm)
                    .on_click(cx.listener(Self::refresh_tree)),
            )
            .child(
                Btn::new("files-save")
                    .label("Save")
                    .icon(IconName::Check)
                    .variant(if self.dirty {
                        Variant::Primary
                    } else {
                        Variant::Ghost
                    })
                    .size(BtnSize::Sm)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.save(&SaveFile, window, cx);
                    })),
            );

        let header = div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(space::S4))
            .pt(px(space::S3))
            .pb(px(space::S2))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::HALF))
                    .child(
                        div()
                            .text_role(&t, TextRole::Display)
                            .text_color(t.fg_strong())
                            .child("Files"),
                    )
                    .child(
                        div()
                            .text_role(&t, TextRole::Xs)
                            .text_color(t.fg_subtle())
                            .child(SharedString::from(self.root.to_string_lossy().into_owned())),
                    ),
            )
            .child(tools);

        let body = h_resizable("files-split")
            .child(
                resizable_panel()
                    .size(px(TREE_WIDTH))
                    .size_range(px(TREE_MIN)..px(TREE_MAX))
                    .child(self.render_tree(&t, cx)),
            )
            .child(
                resizable_panel().child(
                    div()
                        .id("files-editor")
                        .flex()
                        .flex_col()
                        .size_full()
                        .child(
                            div()
                                .flex_1()
                                .overflow_hidden()
                                .child(self.render_editor(&t, cx)),
                        )
                        .when(show_problems, |el| el.child(self.render_problems(&t, cx)))
                        .automation_named(Meta::labelled(Role::Text, "editor"), "files-editor"),
                ),
            );

        div()
            .id("files-page")
            .size_full()
            .key_context("Files")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::save))
            .flex()
            .flex_col()
            .bg(t.frame())
            .child(header)
            .child(div().flex_1().overflow_hidden().child(body))
            .child(self.render_status(&t, cx))
            .automation(Meta::labelled(Role::Other, "files page"))
    }
}

/// `⌘S` in the Files page saves the open buffer (format-on-save first).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = oino)]
pub struct SaveFile;

/// Binds the page's keys (called from `views::init` — the action stays local
/// to the "Files" key context so nothing else sees it).
pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("ctrl-s", SaveFile, Some("Files"))]);
    #[cfg(target_os = "macos")]
    cx.bind_keys([KeyBinding::new("cmd-s", SaveFile, Some("Files"))]);
}
