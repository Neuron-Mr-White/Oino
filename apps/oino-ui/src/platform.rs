//! Platform services the desktop binary plugs in (OS notifications). The shared views call
//! these; when no integration is installed (mobile, tests) they report `false` and the caller
//! falls back to an in-window toast.

use std::sync::Arc;

use gpui_kit::{App, Global};

/// Whether the embedded browser can run in this process (desktop sets it: `browser` feature
/// on and, on Linux, an X11/XWayland window).
#[derive(Clone, Copy, Debug, Default)]
pub struct BrowserSupport(pub bool);

impl Global for BrowserSupport {}

impl BrowserSupport {
    /// Whether links open in the side-pane Browser tab.
    #[must_use]
    pub fn get(cx: &App) -> bool {
        cfg!(feature = "browser") && cx.try_global::<Self>().is_some_and(|b| b.0)
    }
}

/// Posts an OS notification `(title, body)`.
pub type NotifyFn = Arc<dyn Fn(String, String) + Send + Sync>;

/// The installed OS notifier (a GPUI global).
#[derive(Clone)]
pub struct OsNotifier(pub NotifyFn);

impl Global for OsNotifier {}

impl OsNotifier {
    /// Installs `notify` as the OS notifier.
    pub fn install(cx: &mut App, notify: NotifyFn) {
        cx.set_global(Self(notify));
    }

    /// Posts an OS notification; `false` when no notifier is installed.
    pub fn post(cx: &App, title: impl Into<String>, body: impl Into<String>) -> bool {
        match cx.try_global::<Self>() {
            Some(n) => {
                (n.0)(title.into(), body.into());
                true
            }
            None => false,
        }
    }
}
