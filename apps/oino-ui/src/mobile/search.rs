//! M7 Search (design.md §6.7): a full-screen page from ⌕ — field at the top (auto-focus),
//! scope Segmented (All / Actions / Sessions / Files), the palette's sections and fuzzy
//! groups (`model::palette`), rows 56.

use std::rc::Rc;

use gpui_kit::Focusable as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::Input;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    px,
};

use crate::components::controls::segmented;
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::tokens::touch;
use crate::design::{TextRole, size, space};
use crate::host::ThemePref;
use crate::mobile::shell::{MobileShell, Page, SettingsPage, now_ms};
use crate::model::palette::{self, Kind, PaletteAction, PaletteItem, Row, Scope, Section};
use crate::model::sessions::{SessionEntry, age, workspace_label};
use crate::model::timeline::file_changes;
use crate::views::session_pane::SideTab;

fn glyph(stem: &str) -> IconName {
    match stem {
        "square-pen" => IconName::SquarePen,
        "settings" => IconName::Settings,
        "archive" => IconName::Archive,
        "git-fork" => IconName::GitFork,
        "list-tree" => IconName::ListTree,
        "square" => IconName::Square,
        "panel-right" => IconName::PanelRight,
        "file" => IconName::FileText,
        "kanban" => IconName::Kanban,
        "zap" => IconName::Zap,
        "keyboard" => IconName::Keyboard,
        "sun" => IconName::Sun,
        "moon" => IconName::Moon,
        "monitor" => IconName::Monitor,
        "blocks" => IconName::Blocks,
        "refresh-cw" => IconName::RefreshCw,
        "smartphone" => IconName::Smartphone,
        "dot" => IconName::MessageSquareDashed,
        _ => IconName::ChevronRight,
    }
}

impl MobileShell {
    fn search_items(&self, cx: &App) -> Vec<PaletteItem> {
        let a = |label: &str, section: Section, icon: &'static str, id: &str| {
            PaletteItem::new(
                label,
                section,
                Kind::Action,
                icon,
                PaletteAction::App(id.to_owned()),
            )
        };
        let mut items = Vec::new();
        let pane = self.active_pane().map(|p| p.read(cx));
        if let Some(p) = pane {
            for change in file_changes(p.model.transcript.rows()) {
                items.push(
                    PaletteItem::new(
                        change.path.clone(),
                        Section::RecentChanges,
                        Kind::File,
                        "file",
                        PaletteAction::File(change.path.clone()),
                    )
                    .detail(format!("+{} −{}", change.added, change.removed)),
                );
            }
        }
        let mut recent: Vec<SessionEntry> = self
            .sessions
            .groups()
            .into_iter()
            .flat_map(|g| g.sessions)
            .collect();
        recent.sort_by_key(|e| std::cmp::Reverse(e.summary.modified_ms));
        for e in recent {
            items.push(
                PaletteItem::new(
                    if e.summary.title.is_empty() {
                        "Untitled".to_owned()
                    } else {
                        e.summary.title.clone()
                    },
                    Section::RecentSessions,
                    Kind::Session,
                    "dot",
                    PaletteAction::Session(e.summary.id.clone()),
                )
                .detail(format!(
                    "{} · {}",
                    workspace_label(&e.summary.cwd),
                    age(now_ms(), e.summary.modified_ms)
                )),
            );
        }
        items.push(a(
            "New session",
            Section::Suggested,
            "square-pen",
            "new-session",
        ));
        items.push(a("Settings", Section::Suggested, "settings", "settings"));
        items.push(a("Hosts", Section::Suggested, "smartphone", "hosts"));
        if pane.is_some() {
            items.push(a("Compact", Section::Chat, "archive", "compact"));
            items.push(a("Fork", Section::Chat, "git-fork", "fork"));
            items.push(a("Tree", Section::Chat, "list-tree", "tree"));
            items.push(a("Stop", Section::Chat, "square", "abort"));
            items.push(a(
                "Review changes",
                Section::Panels,
                "panel-right",
                "review",
            ));
        }
        if pane.is_some_and(|p| p.has_command("kanboard")) {
            items.push(a("Board", Section::Panels, "kanban", "board"));
        }
        if let Some(p) = pane {
            for (name, description) in p.commands() {
                let mut item = PaletteItem::new(
                    format!("/{name}"),
                    Section::Extensions,
                    Kind::Action,
                    "zap",
                    PaletteAction::Slash(name.clone()),
                );
                if let Some(d) = description {
                    item = item.detail(d.clone());
                }
                items.push(item);
            }
            for s in &p.model.ui.shortcuts {
                items.push(PaletteItem::new(
                    s.description.clone(),
                    Section::Extensions,
                    Kind::Action,
                    "keyboard",
                    PaletteAction::Shortcut(s.id.clone()),
                ));
            }
        }
        items.push(a("Theme: Light", Section::App, "sun", "theme-light"));
        items.push(a("Theme: Dark", Section::App, "moon", "theme-dark"));
        items.push(a("Theme: System", Section::App, "monitor", "theme-system"));
        items.push(a("Extensions", Section::App, "blocks", "extensions"));
        items.push(a(
            "Refresh session list",
            Section::App,
            "refresh-cw",
            "refresh",
        ));
        items
    }

    fn run_search(&mut self, action: PaletteAction, window: &mut Window, cx: &mut Context<Self>) {
        window.blur(cx);
        // Leave the search page first; the action may push its own page.
        if self.page() == &Page::Search {
            self.stack.pop();
        }
        match action {
            PaletteAction::Session(id) => self.open_session(&id, window, cx),
            PaletteAction::Slash(name) => {
                let cmd = format!("/{name}");
                self.with_pane(cx, |p, cx| p.run_prompt(&cmd, cx));
            }
            PaletteAction::Shortcut(id) => self.with_pane(cx, |p, cx| p.invoke_shortcut(&id, cx)),
            PaletteAction::File(path) => {
                let text = format!("@{path} ");
                self.with_pane(cx, |p, cx| p.set_composer_text(&text, cx));
            }
            PaletteAction::App(id) => match id.as_str() {
                "new-session" => self.new_session(None, window, cx),
                "settings" => self.open_settings(SettingsPage::Root, cx),
                "extensions" => self.open_settings(SettingsPage::Extensions, cx),
                "hosts" => self.open_settings(SettingsPage::Remote, cx),
                "compact" => self.with_pane(cx, crate::views::session_pane::SessionPane::compact),
                "fork" => self.with_pane(cx, crate::views::session_pane::SessionPane::fork_session),
                "abort" => self.with_pane(cx, crate::views::session_pane::SessionPane::abort),
                "tree" => self.with_pane(cx, |p, cx| p.open_side(SideTab::Tree, cx)),
                "review" => self.with_pane(cx, |p, cx| p.open_side(SideTab::Review, cx)),
                "board" => {
                    if let Some(key) = self.active_key() {
                        self.open_board(key, true, cx);
                    }
                }
                "theme-light" => self.set_theme(ThemePref::Light, window, cx),
                "theme-dark" => self.set_theme(ThemePref::Dark, window, cx),
                "theme-system" => self.set_theme(ThemePref::System, window, cx),
                "refresh" => self.pull_refresh(cx),
                _ => {}
            },
        }
        cx.notify();
    }

    pub(crate) fn render_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let focus = self.search.read(cx).focus_handle(cx);
        if !focus.is_focused(window) && self.sheet.is_none() {
            self.search.update(cx, |s, cx| s.focus(window, cx));
        }
        let query = self.search.read(cx).value().to_string();
        let items = self.search_items(cx);
        let rows = palette::rows(&items, &query, self.scope);
        let active = Scope::ALL
            .iter()
            .position(|s| *s == self.scope)
            .unwrap_or(0);
        let labels: Vec<&str> = Scope::ALL.iter().map(|s| s.label()).collect();
        let entity = cx.entity().downgrade();
        let scopes = segmented(
            "m-scope",
            &t,
            &labels,
            active,
            Rc::new(move |i, _, cx| {
                let _ = entity.update(cx, |this, cx| {
                    this.scope = Scope::ALL[i];
                    cx.notify();
                });
            }),
        )
        .h(px(touch::SEGMENT));
        let bar = div()
            .flex()
            .items_center()
            .gap(px(space::S1))
            .h(px(touch::APP_BAR))
            .px(px(space::S1))
            .flex_none()
            .child(Self::back_btn(cx))
            .child(
                div()
                    .flex_1()
                    .pr(px(space::S3))
                    .text_size(px(touch::EDIT_FONT))
                    .child(Input::new(&self.search).cleanable(true).large()),
            );
        let list: Vec<AnyElement> = rows
            .iter()
            .enumerate()
            .map(|(i, row)| match row {
                Row::Header(label) => Self::section(&t, *label),
                Row::More(kind, n) => {
                    let scope = match kind {
                        Kind::Action => Scope::Actions,
                        Kind::Session => Scope::Sessions,
                        Kind::File => Scope::Files,
                    };
                    div()
                        .id(ElementId::NamedInteger("m-more".into(), i as u64))
                        .flex()
                        .items_center()
                        .min_h(px(touch::TARGET))
                        .px(px(space::S4))
                        .text_role(&t, TextRole::Sm)
                        .text_color(t.primary())
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.scope = scope;
                            cx.notify();
                        }))
                        .child(SharedString::from(format!("Show {n} more")))
                        .into_any_element()
                }
                Row::Item(ix) => {
                    let item = &items[*ix];
                    let action = item.action.clone();
                    Self::list_row(
                        &t,
                        ElementId::NamedInteger("m-hit".into(), i as u64),
                        Some(glyph(item.icon)),
                        item.label.clone(),
                        item.detail.clone(),
                        None,
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.run_search(action.clone(), window, cx);
                    }))
                    .into_any_element()
                }
            })
            .collect();
        let empty = list.is_empty();
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(bar)
            .child(
                div()
                    .px(px(space::S4))
                    .pb(px(space::S2))
                    .flex_none()
                    .child(scopes),
            )
            .child(
                div()
                    .id("m-search-results")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .border_t_1()
                    .border_color(t.border())
                    .when(empty, |el| {
                        el.child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap(px(space::S2))
                                .pt(px(space::S12))
                                .text_color(t.fg_subtle())
                                .child(icon(IconName::Search, size::ICON_LG, t.fg_subtlest()))
                                .child("No results"),
                        )
                    })
                    .children(list),
            )
            .into_any_element()
    }
}
