//! Command results (browser-control.md §5.3–5.5).

use serde::{Deserialize, Serialize};

/// Whether a failed or cancelled action may have changed the page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideEffect {
    /// Nothing was dispatched.
    #[default]
    None,
    /// Dispatched without proof of effect.
    Uncertain,
    /// The action happened.
    Done,
}

/// Error codes (§5.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserErrorCode {
    /// Browser control is off or the browser could not start (see the message).
    BackendUnavailable,
    /// Several tabs; name one.
    TabRequired,
    /// No such tab in this scope.
    TabNotFound,
    /// Unknown ref.
    RefNotFound,
    /// The ref's element is gone.
    RefStale,
    /// Ambiguous target.
    NotUnique,
    /// Not visible.
    NotVisible,
    /// Disabled.
    NotEnabled,
    /// Not an editable element.
    NotEditable,
    /// Another element covers the target.
    Obscured,
    /// URL refused by policy.
    NavigationBlocked,
    /// A JS dialog is open.
    DialogOpen,
    /// The user has control of the tab.
    TakenOver,
    /// Declined by the user or policy.
    PermissionDenied,
    /// Timed out.
    Timeout,
    /// Cancelled.
    Cancelled,
    /// Anything else (CDP failure, bad arguments).
    ExecutionError,
    /// Host → bundled guest only: ask the user, then resend with `approval` (never reaches the
    /// model; the guest turns a refusal into `permission_denied`).
    ApprovalRequired,
}

impl BrowserErrorCode {
    /// Wire name (`ref_stale`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BackendUnavailable => "backend_unavailable",
            Self::TabRequired => "tab_required",
            Self::TabNotFound => "tab_not_found",
            Self::RefNotFound => "ref_not_found",
            Self::RefStale => "ref_stale",
            Self::NotUnique => "not_unique",
            Self::NotVisible => "not_visible",
            Self::NotEnabled => "not_enabled",
            Self::NotEditable => "not_editable",
            Self::Obscured => "obscured",
            Self::NavigationBlocked => "navigation_blocked",
            Self::DialogOpen => "dialog_open",
            Self::TakenOver => "taken_over",
            Self::PermissionDenied => "permission_denied",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::ExecutionError => "execution_error",
            Self::ApprovalRequired => "approval_required",
        }
    }
}

/// One thing a command caused (§7.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolEvent {
    /// A popup / new tab opened.
    Popup {
        /// The new tab.
        tab: String,
    },
    /// The main frame navigated.
    Navigated {
        /// Redacted URL.
        url: String,
    },
    /// A JS dialog opened.
    Dialog {
        /// `alert|confirm|prompt|beforeunload`.
        dialog: String,
        /// Its message.
        message: String,
    },
    /// A download started.
    Download {
        /// Suggested file name.
        name: String,
    },
    /// Uncaught page exceptions during the command.
    Errors {
        /// Count.
        count: u32,
    },
}

/// `details.browser`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserDetails {
    /// Profile.
    pub profile: String,
    /// Engine kind (`chrome`, `cloak`).
    pub engine: String,
    /// Browser process generation.
    pub generation: u64,
    /// Tab the command ran on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<String>,
    /// The scope's open tabs.
    #[serde(default)]
    pub open_tabs: Vec<String>,
    /// Redacted URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Tab lifecycle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<String>,
    /// Snapshot id (`s5`) when the result carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    /// Caused events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<ToolEvent>,
    /// Effect of the command.
    pub side_effect: SideEffect,
}

/// `details.error`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorDetails {
    /// Code.
    pub code: BrowserErrorCode,
    /// Message with a recovery hint.
    pub message: String,
    /// Whether the page may have changed.
    pub side_effect: SideEffect,
    /// `backend_unavailable` reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `details` of every browser tool result.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserToolDetails {
    /// Tab/page facts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser: Option<BrowserDetails>,
    /// Failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorDetails>,
    /// Op-specific data (status, tab list, downloads, eval value, turn-shot file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// An image produced by a command (screenshots).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserImage {
    /// `image/png` or `image/jpeg`.
    pub mime_type: String,
    /// Bytes (base64 on the wire).
    #[serde(with = "crate::view::b64")]
    pub data: Vec<u8>,
    /// Absolute path of the saved original.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// One command's outcome.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserResult {
    /// Text the model reads.
    pub text: String,
    /// Structured details for renderers and frontends.
    pub details: BrowserToolDetails,
    /// The command failed.
    #[serde(default)]
    pub is_error: bool,
    /// Screenshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<BrowserImage>,
}

impl BrowserResult {
    /// A failure result: `"<code>: <message> (<side effect>)"`.
    #[must_use]
    pub fn error(code: BrowserErrorCode, message: impl Into<String>, side: SideEffect) -> Self {
        let message = message.into();
        let effect = match side {
            SideEffect::None => "no side effect",
            SideEffect::Uncertain => "the action may have happened",
            SideEffect::Done => "the action happened",
        };
        Self {
            text: format!("{}: {message} ({effect})", code.as_str()),
            details: BrowserToolDetails {
                browser: None,
                error: Some(ErrorDetails {
                    code,
                    message,
                    side_effect: side,
                    reason: None,
                }),
                data: None,
            },
            is_error: true,
            image: None,
        }
    }

    /// A plain successful result.
    #[must_use]
    pub fn ok(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    /// `backend_unavailable{reason}`.
    #[must_use]
    pub fn unavailable(reason: &str, message: impl Into<String>) -> Self {
        let mut r = Self::error(
            BrowserErrorCode::BackendUnavailable,
            message,
            SideEffect::None,
        );
        if let Some(e) = &mut r.details.error {
            e.reason = Some(reason.to_owned());
        }
        r
    }

    /// The error code, if failed.
    #[must_use]
    pub fn code(&self) -> Option<BrowserErrorCode> {
        self.details.error.as_ref().map(|e| e.code)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn error_shape() {
        let r = BrowserResult::error(
            BrowserErrorCode::RefStale,
            "e7 is gone; take a new snapshot.",
            SideEffect::None,
        );
        assert_eq!(
            r.text,
            "ref_stale: e7 is gone; take a new snapshot. (no side effect)"
        );
        assert_eq!(
            serde_json::to_value(&r.details).unwrap(),
            json!({"error":{"code":"ref_stale","message":"e7 is gone; take a new snapshot.","sideEffect":"none"}})
        );
        let back: BrowserResult =
            serde_json::from_value(serde_json::to_value(&r).unwrap()).unwrap();
        assert_eq!(back, r);
    }
}
