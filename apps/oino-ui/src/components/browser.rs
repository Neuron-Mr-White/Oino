//! Embedded browser (design.md A1.4): the side pane's **Browser** tab over `gpui-webview`
//! (Wry: `WKWebView` on macOS, `WebView2` on Windows, `WebKitGTK` on Linux X11/XWayland).
//!
//! URL bar, back / forward / reload, open externally, devtools in debug builds. Links from the
//! transcript, `openUrl` from extensions, terminal ⌘-clicks and the `localhost:*` preview chip
//! all land here. Without composition support the native view covers GPUI content in its
//! bounds, so the workspace hides it whenever an overlay (palette, dialog, other route) is up
//! ([`BrowserView::set_shown`]).

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Task, Window, div, px,
};

use crate::components::button::{Btn, BtnSize};
use crate::components::empty::empty_state;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, size, space};

/// Emitted to the pane.
#[derive(Clone, Debug)]
pub enum BrowserEvent {
    /// The page title / URL changed (tab title refresh).
    Changed,
}

/// Normalises what the user typed into a URL (`localhost:3000` → `http://localhost:3000`,
/// bare words → a search).
#[must_use]
pub fn normalise_url(input: &str) -> String {
    let s = input.trim();
    if s.contains("://") || s.starts_with("about:") || s.starts_with("data:") {
        return s.to_owned();
    }
    let host = s.split(['/', '?', '#']).next().unwrap_or_default();
    let local =
        host.starts_with("localhost") || host.starts_with("127.") || host.starts_with("0.0.0.0");
    if local {
        return format!("http://{s}");
    }
    if !s.contains(' ') && host.contains('.') {
        return format!("https://{s}");
    }
    let q: String = s.chars().map(|c| if c == ' ' { '+' } else { c }).collect();
    format!("https://duckduckgo.com/?q={q}")
}

/// Local dev-server URLs (`http://localhost:5173/`, `127.0.0.1:8080`) found in `text`.
#[must_use]
pub fn local_urls(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in
        text.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '(' | ')' | '<' | '>'))
    {
        let w = word.trim_end_matches(['.', ',', ';']);
        let bare = w
            .trim_start_matches("http://")
            .trim_start_matches("https://");
        let is_local = ["localhost:", "127.0.0.1:", "0.0.0.0:"]
            .iter()
            .any(|p| bare.starts_with(p))
            && bare
                .split(':')
                .nth(1)
                .is_some_and(|port| port.chars().next().is_some_and(|c| c.is_ascii_digit()));
        if is_local {
            let url = if w.contains("://") {
                w.to_owned()
            } else {
                format!("http://{w}")
            };
            let url = url.replace("0.0.0.0", "localhost");
            if !out.contains(&url) {
                out.push(url);
            }
        }
    }
    out
}

/// One browser tab.
pub struct BrowserView {
    url: String,
    title: String,
    input: Entity<InputState>,
    web: Option<Entity<gpui_webview::WebView>>,
    error: Option<String>,
    shown: bool,
    _subs: Vec<Subscription>,
    loads: Option<Task<()>>,
}

impl EventEmitter<BrowserEvent> for BrowserView {}

impl BrowserView {
    /// A browser that loads `url` when first shown.
    pub fn new(url: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let url = normalise_url(url);
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search or enter an address")
                .default_value(SharedString::from(url.clone()))
        });
        let sub = cx.subscribe_in(&input, window, |this, input, e: &InputEvent, _, cx| {
            if matches!(e, InputEvent::PressEnter { .. }) {
                let typed = input.read(cx).value().to_string();
                this.load(&typed, cx);
            }
        });
        Self {
            url,
            title: String::new(),
            input,
            web: None,
            error: None,
            shown: true,
            _subs: vec![sub],
            loads: None,
        }
    }

    /// Tab title: the page title, else the host.
    #[must_use]
    pub fn title(&self) -> String {
        if !self.title.is_empty() {
            return self.title.clone();
        }
        let host = self
            .url
            .split("://")
            .nth(1)
            .and_then(|r| r.split('/').next())
            .unwrap_or("Browser");
        host.to_owned()
    }

    /// The current URL.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Navigates to `input` (normalised).
    pub fn load(&mut self, input: &str, cx: &mut Context<Self>) {
        self.url = normalise_url(input);
        if let Some(web) = &self.web {
            let url = self.url.clone();
            web.update(cx, |w, _| w.load_url(&url));
        }
        cx.notify();
    }

    /// Shows or hides the native view (the workspace calls this every frame; native calls
    /// happen only on change).
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.shown == shown {
            return;
        }
        self.shown = shown;
        if let Some(web) = &self.web {
            web.update(cx, |w, _| {
                if shown {
                    w.show();
                } else {
                    w.hide();
                }
            });
        }
    }

    fn build(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<(bool, String)>();
        let tx_title = tx.clone();
        let t = OinoTheme::get(cx);
        let [r, g, b, _] = t.p.frame.to_be_bytes();
        let builder = wry::WebViewBuilder::new()
            .with_url(self.url.clone())
            .with_background_color((r, g, b, 0xFF))
            .with_devtools(cfg!(debug_assertions))
            .with_on_page_load_handler(move |_, url| {
                let _ = tx.unbounded_send((false, url));
            })
            .with_document_title_changed_handler(move |title| {
                let _ = tx_title.unbounded_send((true, title));
            });
        match gpui_webview::WebView::build(builder, window, cx) {
            Ok(web) => {
                let _ = web.set_theme(if t.dark {
                    wry::Theme::Dark
                } else {
                    wry::Theme::Light
                });
                let web = cx.new(|_| web);
                if !self.shown {
                    web.update(cx, |w, _| w.hide());
                }
                self.web = Some(web);
                self.loads = Some(cx.spawn_in(window, async move |this, cx| {
                    use futures::StreamExt as _;
                    while let Some((is_title, value)) = rx.next().await {
                        let alive = this.update_in(cx, |v, window, cx| {
                            if is_title {
                                v.title = value;
                            } else {
                                v.url.clone_from(&value);
                                v.input.update(cx, |s, cx| s.set_value(value, window, cx));
                            }
                            cx.emit(BrowserEvent::Changed);
                            cx.notify();
                        });
                        if alive.is_err() {
                            break;
                        }
                    }
                }));
            }
            Err(error) => {
                tracing::warn!(%error, "embedded browser unavailable");
                self.error = Some(error.to_string());
            }
        }
    }
}

impl Render for BrowserView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = OinoTheme::get(cx);
        if self.web.is_none() && self.error.is_none() && self.shown {
            self.build(window, cx);
        }
        let has_web = self.web.is_some();
        let nav = |id: &'static str, glyph: IconName, tip: &'static str| {
            Btn::icon_only(id, glyph, tip)
                .size(BtnSize::Sm)
                .disabled(!has_web)
        };
        let web_back = self.web.clone();
        let web_fwd = self.web.clone();
        let web_reload = self.web.clone();
        let web_dev = self.web.clone();
        let external = self.url.clone();
        let bar = div()
            .flex()
            .items_center()
            .gap(px(space::S1))
            .h(px(size::CONTROL_LG + space::S1))
            .px(px(space::S2))
            .border_b_1()
            .border_color(t.border())
            .child(
                nav("br-back", IconName::ArrowLeft, "Back").on_click(move |_, _, cx| {
                    if let Some(w) = &web_back {
                        w.update(cx, |w, _| {
                            let _ = w.back();
                        });
                    }
                }),
            )
            .child(
                nav("br-fwd", IconName::ArrowRight, "Forward").on_click(move |_, _, cx| {
                    if let Some(w) = &web_fwd {
                        w.update(cx, |w, _| {
                            let _ = w.forward();
                        });
                    }
                }),
            )
            .child(
                nav("br-reload", IconName::RotateCw, "Reload").on_click(move |_, _, cx| {
                    if let Some(w) = &web_reload {
                        w.update(cx, |w, _| {
                            let _ = w.raw().reload();
                        });
                    }
                }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_role(&t, TextRole::Sm)
                    .child(Input::new(&self.input)),
            )
            .when(cfg!(debug_assertions), |el| {
                el.child(nav("br-dev", IconName::Code, "Developer tools").on_click(
                    move |_, _, cx| {
                        if let Some(w) = &web_dev {
                            w.update(cx, |w, _| w.raw().open_devtools());
                        }
                    },
                ))
            })
            .child(
                Btn::icon_only(
                    "br-ext",
                    IconName::ExternalLink,
                    "Open in the system browser",
                )
                .size(BtnSize::Sm)
                .on_click(move |_, _, cx| cx.open_url(&external)),
            );
        let body = match (&self.web, &self.error) {
            (Some(web), _) => div().flex_1().min_h_0().child(web.clone()).into_any_element(),
            (None, Some(error)) => div()
                .flex_1()
                .p(px(space::S4))
                .child(empty_state(
                    &t,
                    IconName::Globe,
                    "The embedded browser is unavailable",
                    Some(
                        format!(
                            "{error}. On Linux it needs X11 or XWayland: turn off \"Native Wayland\" in Settings → Appearance, or open the page in the system browser."
                        )
                        .into(),
                    ),
                    Vec::new(),
                ))
                .into_any_element(),
            (None, None) => div().flex_1().into_any_element(),
        };
        div().flex().flex_col().size_full().child(bar).child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_normalise() {
        assert_eq!(normalise_url("localhost:5173"), "http://localhost:5173");
        assert_eq!(normalise_url("docs.rs/gpui"), "https://docs.rs/gpui");
        assert_eq!(normalise_url("https://x.dev"), "https://x.dev");
        assert_eq!(
            normalise_url("rust ownership"),
            "https://duckduckgo.com/?q=rust+ownership"
        );
    }

    #[test]
    fn finds_local_dev_servers() {
        let out = "  VITE ready\n  ➜  Local:   http://localhost:5173/\n  serving on 0.0.0.0:8080.";
        assert_eq!(
            local_urls(out),
            vec![
                "http://localhost:5173/".to_owned(),
                "http://localhost:8080".to_owned()
            ]
        );
        assert!(local_urls("see https://example.com:443").is_empty());
    }
}
