//! The central host switcher (round5 #12, §5): one chip at the top of the sidebar reading
//! the active host — exactly "Current PC" when no hosts are configured — with a monitor
//! icon and a chevron. Its menu lists the configured hosts (name + SSH target, checked
//! when active) with "Current PC" always present and "Add host…" opening a small form.
//! Modelled on ZCode's centralized identity; pages never pick hosts (the composer's
//! project header is gone with #3, the board keeps its project chip).
//!
//! Remote selection is honest this round: `oino serve` does not exist yet (STATUS known
//! issue 4), so picking a host stores the selection and toasts that the app keeps running
//! locally. Hosts persist in `hosts.json` next to `desktop.json` (own store file, same
//! shape as the mobile host list); an empty or missing file means "Current PC".

use std::path::Path;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, px,
};
use serde::{Deserialize, Serialize};

use crate::automation::Automatable as _;
use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::dialog::{OnClose, modal_frame};
use crate::components::menu::{MenuEntry, Placement, menu, popover};
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::views::shell::AppShell;

/// The always-present local entry.
pub(crate) const CURRENT_PC: &str = "Current PC";

/// One configured remote host (`hosts.json`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostEntry {
    /// Display name.
    pub name: String,
    /// SSH target (`user@host`).
    pub target: String,
}

/// The host store next to `desktop.json` (`~/.oino/hosts.json`). Missing or empty means
/// the app runs on this PC only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HostsFile {
    /// Configured hosts (name order as added).
    pub hosts: Vec<HostEntry>,
    /// The selected host's name; `None` = this PC.
    pub active: Option<String>,
}

impl HostsFile {
    /// Reads `agent_dir/hosts.json`; a missing or malformed file is the empty default.
    #[must_use]
    pub fn load(agent_dir: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(agent_dir.join("hosts.json")) else {
            return Self::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    /// Writes the store (best effort; the switcher still works without persistence).
    pub fn save(&self, agent_dir: &Path) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(agent_dir.join("hosts.json"), json);
        }
    }

    /// The chip label: the active host's name, else [`CURRENT_PC`].
    #[must_use]
    pub fn active_label(&self) -> &str {
        self.active
            .as_deref()
            .filter(|name| self.hosts.iter().any(|h| h.name.as_str() == *name))
            .unwrap_or(CURRENT_PC)
    }
}

/// The "Add host…" form state.
pub(crate) struct HostForm {
    pub(crate) name: Entity<InputState>,
    pub(crate) target: Entity<InputState>,
}

impl AppShell {
    /// Selects a host (or this PC with `None`), persists the choice and — for a remote —
    /// toasts the honest scope: nothing attaches until `oino serve` exists.
    pub(crate) fn select_host(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        let remote = name.is_some();
        self.hosts.active = name;
        self.hosts.save(&self.host.info().agent_dir);
        if remote {
            self.toast(
                crate::components::toast::ToastKind::Info,
                None,
                "Remote agents need `oino serve` (coming later) — staying on Current PC",
                cx,
            );
        }
        cx.notify();
    }

    /// Opens the "Add host…" form.
    pub(crate) fn open_host_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("My server"));
        let target = cx.new(|cx| InputState::new(window, cx).placeholder("user@host"));
        name.update(cx, |s, cx| s.focus(window, cx));
        self.host_form = Some(HostForm { name, target });
        cx.notify();
    }

    /// Saves the "Add host…" form (a name and an SSH target are required).
    pub(crate) fn save_host_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &self.host_form else {
            return;
        };
        let name = form.name.read(cx).value().trim().to_owned();
        let target = form.target.read(cx).value().trim().to_owned();
        if name.is_empty() || target.is_empty() {
            self.toast(
                crate::components::toast::ToastKind::Warning,
                None,
                "A name and an SSH target are needed",
                cx,
            );
            return;
        }
        self.hosts.hosts.retain(|h| h.name != name);
        self.hosts.hosts.push(HostEntry {
            name: name.clone(),
            target,
        });
        self.hosts.save(&self.host.info().agent_dir);
        self.host_form = None;
        self.toast(
            crate::components::toast::ToastKind::Info,
            None,
            format!("Host {name} added"),
            cx,
        );
        cx.notify();
    }

    /// The sidebar-top host chip and its menu (#12).
    pub(crate) fn render_host_switcher(
        &mut self,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = self.hosts.active_label();
        let menu_open = self.host_menu;
        let e = cx.entity().downgrade();
        let chip = div()
            .id("host-switcher")
            .flex()
            .items_center()
            .gap(px(space::S2))
            .h(px(size::SEARCH))
            .px(px(space::S2_5))
            .mb(px(space::S2))
            .rounded(px(radius::R_ROW))
            .bg(t.chip())
            .border_1()
            .border_color(t.border())
            .cursor_pointer()
            .text_role(t, TextRole::Base)
            .when(menu_open, |el| el.border_color(t.border_hover()))
            .when(!menu_open, |el| el.hover(|s| s.border_color(t.border_hover())))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.host_menu = !this.host_menu;
                cx.notify();
            }))
            .child(icon(IconName::Monitor, size::ICON, t.fg_subtle()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .weight_medium()
                    .text_color(t.fg())
                    .child(SharedString::from(label.to_owned())),
            )
            .child(icon(IconName::ChevronDown, size::ICON_MD, t.fg_subtlest()))
            .automation(
                crate::automation::Meta::labelled(
                    crate::automation::Role::Chip,
                    SharedString::from(label.to_owned()),
                )
                .selected(menu_open),
            );
        let close = move |_: &mut Window, cx: &mut App| {
            let _ = e.update(cx, |ws, cx| {
                ws.host_menu = false;
                cx.notify();
            });
        };
        popover(
            chip,
            menu_open,
            Placement::BelowStart,
            || self.host_menu_entries(t, cx),
            close,
        )
        .into_any_element()
    }

    /// The host menu: "Current PC" (always), the configured hosts, "Add host…".
    fn host_menu_entries(&self, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        let e = cx.entity().downgrade();
        let mut entries = vec![MenuEntry::Label("Hosts".into(), None)];
        entries.push({
            let e = e.clone();
            MenuEntry::item(CURRENT_PC, move |_, cx| {
                let _ = e.update(cx, |ws, cx| {
                    ws.host_menu = false;
                    ws.select_host(None, cx);
                });
            })
            .icon(IconName::Monitor)
            .checked(self.hosts.active.is_none())
        });
        for host in &self.hosts.hosts {
            let e = e.clone();
            let name = host.name.clone();
            entries.push(
                MenuEntry::item(host.name.clone(), move |_, cx| {
                    let _ = e.update(cx, |ws, cx| {
                        ws.host_menu = false;
                        ws.select_host(Some(name.clone()), cx);
                    });
                })
                .icon(IconName::Cloud)
                .detail(SharedString::from(format!(
                    "{} · not connected",
                    host.target
                )))
                .checked(self.hosts.active.as_deref() == Some(host.name.as_str())),
            );
        }
        entries.push(MenuEntry::Separator);
        entries.push({
            let e = e.clone();
            MenuEntry::item("Add host…", move |window, cx| {
                let _ = e.update(cx, |ws, cx| {
                    ws.host_menu = false;
                    ws.open_host_form(window, cx);
                });
            })
            .icon(IconName::Plus)
        });
        menu("host-menu", t, size::MENU_MIN * 1.4, entries).into_any_element()
    }

    /// The "Add host…" dialog, when open.
    pub(crate) fn render_host_form(
        &self,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let form = self.host_form.as_ref()?;
        let shell = cx.entity().downgrade();
        let close: OnClose = Rc::new(move |_, cx| {
            let _ = shell.update(cx, |ws, cx| {
                ws.host_form = None;
                cx.notify();
            });
        });
        let cancel = Rc::clone(&close);
        let labeled = |label: &str, input: &Entity<InputState>| {
            div()
                .flex()
                .flex_col()
                .gap(px(space::S1))
                .child(
                    div()
                        .text_role(t, TextRole::Xs)
                        .weight_semibold()
                        .text_color(t.fg_subtle())
                        .child(label.to_owned()),
                )
                .child(Input::new(input))
        };
        let body = div()
            .flex()
            .flex_col()
            .gap(px(space::S3))
            .child(labeled("Name", &form.name))
            .child(labeled("SSH target", &form.target))
            .child(
                div()
                    .text_role(t, TextRole::Sm)
                    .text_color(t.fg_subtlest())
                    .child(
                        "Hosts attach once `oino serve` exists; for now the list remembers \
                         them.",
                    ),
            )
            .into_any_element();
        let cancel_btn = Btn::new("host-form-cancel")
            .label("Cancel")
            .size(BtnSize::Xl)
            .on_click(move |_, w, cx| cancel(w, cx))
            .into_any_element();
        let save_btn = Btn::new("host-form-save")
            .label("Save host")
            .variant(Variant::Primary)
            .size(BtnSize::Xl)
            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                this.save_host_form(cx);
            }))
            .into_any_element();
        Some(modal_frame(
            t,
            "host-form",
            "Add host".into(),
            size::DIALOG_SM,
            body,
            vec![cancel_btn, save_btn],
            close,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{CURRENT_PC, HostEntry, HostsFile};
    use crate::model::sessions::project_label;

    #[test]
    fn empty_store_reads_as_current_pc() {
        let hosts = HostsFile::default();
        assert_eq!(hosts.active_label(), CURRENT_PC);
        assert!(hosts.hosts.is_empty());
    }

    #[test]
    fn store_round_trips_and_resolves_the_active_label() {
        let hosts = HostsFile {
            hosts: vec![HostEntry {
                name: "ti".into(),
                target: "ti@ti".into(),
            }],
            active: Some("ti".into()),
        };
        let json = serde_json::to_string(&hosts).unwrap();
        let parsed: HostsFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, hosts);
        assert_eq!(parsed.active_label(), "ti");
        // A stale active name falls back to this PC.
        assert_eq!(
            HostsFile {
                active: Some("gone".into()),
                ..parsed
            }
            .active_label(),
            CURRENT_PC
        );
    }

    #[test]
    fn malformed_files_are_the_default() {
        let dir = std::env::temp_dir().join(format!("oino-hosts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("hosts.json"), "{not json").unwrap();
        assert_eq!(HostsFile::load(&dir), HostsFile::default());
        let missing = HostsFile::load(&dir.join("absent"));
        assert_eq!(missing.active_label(), CURRENT_PC);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn project_labels_come_from_the_path_tail() {
        assert_eq!(project_label("/home/oi/llm/oino"), "oino");
    }
}
