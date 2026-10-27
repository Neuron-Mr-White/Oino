//! Browser-control wire types (browser-control.md §8.1).
//!
//! One definition shared by the host service (`oino-browser`), the extension contract
//! (`oino-extension-api`), the WASM host, frontends and the `oino.browser` guest. Everything is
//! plain serde data and builds for `wasm32-wasip2`.
//!
//! # API map
//!
//! | Module | Contents |
//! |---|---|
//! | [`command`] | [`BrowserCommand`] (one variant per `browser_*` tool + host-only ops) and its argument enums |
//! | [`result`] | [`BrowserResult`], [`BrowserToolDetails`], [`BrowserErrorCode`], [`SideEffect`], [`BrowserImage`] |
//! | [`status`] | [`BrowserStatus`], [`TabInfo`], [`TabOwner`], [`ProfileInfo`] |
//! | [`view`] | [`BrowserViewEvent`], [`BrowserInput`], [`Viewport`], [`FrameData`] |
//! | [`scope`] | [`BrowserScope`] |

#![forbid(unsafe_code)]

pub mod command;
pub mod result;
pub mod scope;
pub mod status;
pub mod view;

pub use command::{BrowserCommand, HistoryAction, Modifier, MouseButton, TabsAction, WaitLoad};
pub use result::{
    BrowserDetails, BrowserErrorCode, BrowserImage, BrowserResult, BrowserToolDetails,
    ErrorDetails, SideEffect, ToolEvent,
};
pub use scope::BrowserScope;
pub use status::{
    BrowserStatus, ControlledBy, ProfileInfo, TabInfo, TabLifecycle, TabOwner, UnavailableReason,
};
pub use view::{
    BrowserInput, BrowserViewEvent, FrameData, FrontendRequest, KeyEventType, MouseEventType,
    Viewport,
};

/// Removes credentials, query and fragment from `url` (details / UI redaction, §5.4).
#[must_use]
pub fn redact_url(url: &str) -> String {
    let cut = url.find(['?', '#']).map_or(url, |i| &url[..i]);
    match cut.split_once("://") {
        Some((scheme, rest)) => {
            let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
            let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
            format!("{scheme}://{host}{path}")
        }
        None => cut.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::redact_url;

    #[test]
    fn redaction_drops_secrets() {
        assert_eq!(
            redact_url("https://u:p@example.com/a/b?token=1#x"),
            "https://example.com/a/b"
        );
        assert_eq!(redact_url("about:blank"), "about:blank");
        assert_eq!(redact_url("http://localhost:3000"), "http://localhost:3000");
    }
}
