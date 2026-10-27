//! The side pane's **Browser** tab (browser-control.md §9.2, B4/B5).
//!
//! When the host browser service is installed ([`hub::BrowserHub`]), the tab is a
//! [`screencast::ScreencastView`] of this session's tabs (the same Chromium the agent drives,
//! with take-over). Without it, it falls back to the native [`webview::WebBrowser`].
//!
//! | Module | Contents |
//! |---|---|
//! | [`hub`] | [`hub::BrowserHub`] global, per-pane reveal watcher |
//! | [`model`] | pure view model (tabs, page, take-over strip, cards, letterbox) |
//! | [`input`] | GPUI keystrokes / modifiers → `BrowserInput` |
//! | [`screencast`] | the screencast view |
//! | [`webview`] | fallback web view; `normalise_url`, `local_urls` |

pub mod hub;
pub mod input;
pub mod model;
pub mod screencast;
pub mod webview;

use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, Window, div,
};

pub use webview::{BrowserEvent, local_urls, normalise_url};

enum Inner {
    Cast(Entity<screencast::ScreencastView>),
    Web(Entity<webview::WebBrowser>),
}

/// The Browser tab of one session pane.
pub struct BrowserView {
    inner: Inner,
    title: String,
    url: String,
    _sub: Subscription,
}

impl EventEmitter<BrowserEvent> for BrowserView {}

impl BrowserView {
    /// A fallback web view loading `url` (no session).
    pub fn new(url: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let web = cx.new(|cx| webview::WebBrowser::new(url, window, cx));
        let sub = cx.subscribe(&web, |this, web, _: &BrowserEvent, cx| {
            this.title = web.read(cx).title();
            web.read(cx).url().clone_into(&mut this.url);
            cx.emit(BrowserEvent::Changed);
            cx.notify();
        });
        Self {
            inner: Inner::Web(web),
            title: "Browser".into(),
            url: normalise_url(url),
            _sub: sub,
        }
    }

    /// The Browser tab of `session` (workspace `cwd`): screencast when the browser service is
    /// installed, else the fallback web view. `url` other than `about:blank` opens a user tab.
    pub fn for_session(
        session: &str,
        cwd: &str,
        url: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let Some(host) = hub::BrowserHub::host(cx) else {
            return Self::new(url, window, cx);
        };
        if !host.status(None).available {
            return Self::new(url, window, cx);
        }
        let initial = (url != "about:blank").then(|| normalise_url(url));
        let cast =
            cx.new(|cx| screencast::ScreencastView::new(host, session, cwd, initial, window, cx));
        let sub = cx.subscribe(&cast, |this, cast, _: &screencast::CastEvent, cx| {
            let m = &cast.read(cx).model;
            this.title = m.title();
            this.url.clone_from(&m.page.url);
            cx.emit(BrowserEvent::Changed);
            cx.notify();
        });
        Self {
            inner: Inner::Cast(cast),
            title: "Browser".into(),
            url: url.to_owned(),
            _sub: sub,
        }
    }

    /// Tab title: the page title, else the host.
    #[must_use]
    pub fn title(&self) -> String {
        self.title.clone()
    }

    /// The current URL.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Opens `input` (normalised): a new user tab in screencast mode, in place for the web view.
    pub fn load(&mut self, input: &str, cx: &mut Context<Self>) {
        let url = normalise_url(input);
        match &self.inner {
            Inner::Cast(c) => c.update(cx, |c, cx| c.open_user_tab(url, cx)),
            Inner::Web(w) => w.update(cx, |w, cx| w.load(&url, cx)),
        }
    }

    /// Shows or hides the view (native web view visibility / screencast on-off).
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        match &self.inner {
            Inner::Cast(c) => c.update(cx, |c, cx| c.set_shown(shown, cx)),
            Inner::Web(w) => w.update(cx, |w, cx| w.set_shown(shown, cx)),
        }
    }
}

impl Render for BrowserView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let child = match &self.inner {
            Inner::Cast(c) => c.clone().into_any_element(),
            Inner::Web(w) => w.clone().into_any_element(),
        };
        div().size_full().child(child)
    }
}
