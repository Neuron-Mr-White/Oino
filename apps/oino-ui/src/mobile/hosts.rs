//! M10 Remote attach (design.md §6.10, gpui-app §4): the hosts list ("This phone" first,
//! then paired desktops with host, last seen and a status dot) and the "Pair a desktop" page
//! (instruction card — the system camera opens the `oino://` QR link into the app — plus a
//! paste field). Paired hosts persist in `mobile.json` through [`MobilePlatform`] hooks.
//!
//! Attaching to a paired host needs `oino serve` + the RPC backend (modes §5.1); until then a
//! pairing is stored and listed as "not connected" and sessions run on the phone.

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::Input;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, ClickEvent, Context, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    px,
};
use serde::{Deserialize, Serialize};

use crate::components::badge::{DotKind, dot};
use crate::components::button::Variant;
use crate::components::toast::ToastKind;
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::tokens::touch;
use crate::design::{TextRole, radius, size, space};
use crate::mobile::MobilePlatform;
use crate::mobile::shell::{MobileShell, Page, Sheet, now_ms, pill_btn};
use crate::model::sessions::age;

/// A paired desktop (`oino://<host>:<port>?token=…&fp=…`, modes §5.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteHost {
    /// Display name (host name).
    pub name: String,
    /// `host:port`.
    pub address: String,
    /// Certificate fingerprint (`fp`), when given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// Paired at / last seen, Unix milliseconds.
    pub last_seen_ms: i64,
}

impl RemoteHost {
    /// Parses a pairing URL `oino://<host>:<port>?token=<t>&fp=<sha256>`. The token is not
    /// kept here (it belongs in the platform keystore once the RPC backend exists).
    #[must_use]
    pub fn from_url(url: &str, now_ms: i64) -> Option<Self> {
        let rest = url.trim().strip_prefix("oino://")?;
        let (address, query) = rest.split_once('?').unwrap_or((rest, ""));
        let address = address.trim_end_matches('/');
        let (host, port) = address.rsplit_once(':')?;
        if host.is_empty() || port.parse::<u16>().is_err() {
            return None;
        }
        let param = |k: &str| {
            query
                .split('&')
                .filter_map(|kv| kv.split_once('='))
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.to_owned())
        };
        param("token")?;
        Some(Self {
            name: host.to_owned(),
            address: address.to_owned(),
            fingerprint: param("fp"),
            last_seen_ms: now_ms,
        })
    }
}

impl MobileShell {
    /// The hosts list (Settings → Remote).
    pub(crate) fn render_hosts(
        &mut self,
        t: &OinoTheme,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let this_phone = div()
            .flex()
            .items_center()
            .gap(px(space::S3))
            .min_h(px(touch::ROW))
            .px(px(space::S4))
            .child(icon(IconName::Smartphone, size::ICON_LG, t.fg_subtle()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_role(t, TextRole::Base).child("This phone"))
                    .child(
                        div()
                            .text_role(t, TextRole::Sm)
                            .text_color(t.fg_subtle())
                            .child(SharedString::from(format!(
                                "Embedded core · {}",
                                self.host.info().backend
                            ))),
                    ),
            )
            .child(dot(t, DotKind::Unread))
            .into_any_element();
        let mut rows = vec![this_phone];
        for (i, h) in self.hosts.clone().into_iter().enumerate() {
            let address = h.address.clone();
            rows.push(
                Self::list_row(
                    t,
                    ElementId::NamedInteger("m-host".into(), i as u64),
                    Some(IconName::Monitor),
                    h.name.clone(),
                    Some(format!(
                        "{} · paired {} · not connected",
                        h.address,
                        age(now_ms(), h.last_seen_ms)
                    )),
                    Some(div().size(px(size::DOT)).rounded(px(radius::R_FULL)).bg(t.fg_subtlest()).into_any_element()),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let address = address.clone();
                    this.open_sheet(
                        Sheet::Confirm(crate::mobile::shell::Confirm {
                            title: SharedString::from(format!("Forget {address}?")),
                            body: "The pairing is removed from this phone. Pair again from the desktop's QR code.".into(),
                            action: "Forget".into(),
                            run: std::rc::Rc::new(move |this, _, cx| {
                                this.hosts.retain(|h| h.address != address);
                                (MobilePlatform::get(cx).save_hosts)(&this.hosts);
                            }),
                        }),
                        cx,
                    );
                }))
                .into_any_element(),
            );
        }
        vec![
            Self::section(t, "Hosts"),
            Self::card_rows(t, rows),
            div()
                .px(px(space::S4))
                .pt(px(space::S4))
                .child(
                    pill_btn(
                        "m-pair",
                        "Pair a desktop",
                        Some(IconName::Plus),
                        Variant::Outline,
                    )
                    .full()
                    .on_click(cx.listener(|this, _, _, cx| this.push(Page::Pair, cx))),
                )
                .into_any_element(),
        ]
    }

    fn card_rows(t: &OinoTheme, rows: Vec<AnyElement>) -> AnyElement {
        let n = rows.len();
        div()
            .mx(px(space::S4))
            .flex()
            .flex_col()
            .rounded(px(radius::R_CARD))
            .bg(t.card())
            .border_1()
            .border_color(t.card_border())
            .overflow_hidden()
            .children(rows.into_iter().enumerate().map(move |(i, r)| {
                div()
                    .when(i + 1 < n, |el| el.border_b_1().border_color(t.border()))
                    .child(r)
            }))
            .into_any_element()
    }

    fn pair_from_field(&mut self, cx: &mut Context<Self>) {
        let url = self.pair.read(cx).value().to_string();
        match RemoteHost::from_url(&url, now_ms()) {
            Some(host) => {
                self.hosts.retain(|h| h.address != host.address);
                let name = host.name.clone();
                self.hosts.insert(0, host);
                (MobilePlatform::get(cx).save_hosts)(&self.hosts);
                self.toast(ToastKind::Success, None, format!("Paired with {name}"), cx);
                self.stack.pop();
            }
            None => self.toast(
                ToastKind::Error,
                None,
                "Not a pairing link (expected oino://host:port?token=…)",
                cx,
            ),
        }
        cx.notify();
    }

    /// "Pair a desktop" page.
    pub(crate) fn render_pair(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let bar = Self::app_bar(
            &t,
            Some(Self::back_btn(cx)),
            Self::title(&t, "Pair a desktop", None),
            Vec::new(),
        );
        let step = |n: &'static str, text: &'static str| {
            div()
                .flex()
                .items_start()
                .gap(px(space::S3))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_shrink_0()
                        .size(px(size::CONTROL))
                        .rounded(px(radius::R_FULL))
                        .bg(t.accent())
                        .text_role(&t, TextRole::Sm)
                        .weight_medium()
                        .text_color(t.primary())
                        .child(n),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .pt(px(space::HALF))
                        .text_role(&t, TextRole::Base)
                        .text_color(t.fg())
                        .child(text),
                )
        };
        let card = div()
            .mx(px(space::S4))
            .mt(px(space::S4))
            .flex()
            .flex_col()
            .gap(px(space::S4))
            .p(px(space::S4))
            .rounded(px(radius::R_CARD))
            .bg(t.card())
            .border_1()
            .border_color(t.card_border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S2))
                    .child(icon(IconName::Monitor, size::ICON_LG, t.fg_subtle()))
                    .child(
                        div()
                            .text_role(&t, TextRole::Lg)
                            .weight_medium()
                            .child("Scan with your camera"),
                    ),
            )
            .child(step(
                "1",
                "On the desktop open Oino Desktop → Settings → Remote access.",
            ))
            .child(step("2", "Point this phone's camera app at the QR code."))
            .child(step("3", "Tap the oino:// link — Oino opens and pairs."));
        let paste = div()
            .mx(px(space::S4))
            .mt(px(space::S6))
            .flex()
            .flex_col()
            .gap(px(space::S2))
            .child(crate::components::badge::section_label(&t, "Or paste the link"))
            .child(
                div()
                    .text_size(px(touch::EDIT_FONT))
                    .child(Input::new(&self.pair).cleanable(true).large()),
            )
            .child(
                pill_btn("m-pair-go", "Pair", Some(IconName::Check), Variant::Primary)
                    .full()
                    .on_click(cx.listener(|this, _, _, cx| this.pair_from_field(cx))),
            )
            .child(
                div()
                    .text_role(&t, TextRole::Sm)
                    .text_color(t.fg_subtle())
                    .child("Attaching to a desktop needs Oino Desktop's remote access (oino serve). Until then sessions run on this phone."),
            );
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(bar)
            .child(
                div()
                    .id("m-pair-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(card)
                    .child(paste),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_urls_parse_without_keeping_the_token() {
        let h = RemoteHost::from_url("oino://studio.local:7420?token=abc&fp=12ab", 5).unwrap();
        assert_eq!(h.name, "studio.local");
        assert_eq!(h.address, "studio.local:7420");
        assert_eq!(h.fingerprint.as_deref(), Some("12ab"));
        let json = serde_json::to_string(&h).unwrap();
        assert!(!json.contains("abc"));
        assert!(RemoteHost::from_url("oino://studio.local:7420", 0).is_none());
        assert!(RemoteHost::from_url("https://x:1?token=a", 0).is_none());
        assert!(RemoteHost::from_url("oino://host:notaport?token=a", 0).is_none());
    }
}
