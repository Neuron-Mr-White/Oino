//! GPUI views (feature `views`): the desktop shell (design.md §5) over the shared components.
//!
//! | Module | Screen |
//! |---|---|
//! | [`shell`] | D1–D7 frames, routes, palette, toasts ([`Workspace`]) |
//! | [`session_pane`] | one session: state, pump, commands |
//! | [`settings_page`] | D8 Settings + extension settings hub |
//! | `board`, `extensions_page`, `onboarding` | D9, D12, D13 (`impl Workspace`) |

#![allow(
    clippy::too_many_lines,
    clippy::many_single_char_names,
    clippy::similar_names,
    clippy::unused_self,
    clippy::needless_pass_by_value,
    clippy::type_complexity,
    clippy::fn_params_excessive_bools,
    reason = "declarative GPUI element trees (session-bound render helpers stay methods for uniform call sites; callbacks are `Rc` handles cloned into child closures): one long builder chain per screen part reads better than fragments, and layout code uses short names for theme (`t`) and geometry (`w`, `h`, `r`)"
)]

pub mod board;
pub mod extensions_page;
pub mod onboarding;
pub mod session_pane;
pub mod settings_page;
pub mod shell;

use gpui_kit::{App, KeyBinding};

pub use shell::Workspace;

gpui_kit::actions!(
    oino,
    [
        /// New session in the current workspace.
        NewSession,
        /// New session in a chosen folder.
        OpenFolder,
        /// Show/hide the sidebar.
        ToggleSidebar,
        /// Show/hide the side pane.
        ToggleSidePane,
        /// Show/hide the bottom frame.
        ToggleBottom,
        /// Abort the active run.
        Abort,
        /// Command palette.
        CommandPalette,
        /// Model menu.
        PickModel,
        /// Thinking menu.
        PickThinking,
        /// Settings.
        OpenSettings,
        /// Toggle light/dark.
        ToggleTheme,
        /// Split the conversation frame.
        SplitPane,
        /// Session tree in the side pane.
        TreeView,
        /// Find in the conversation.
        FindInConversation,
        /// Close the find bar.
        CloseFind,
        /// Focus the composer.
        FocusComposer,
        /// Expand / collapse the status panel.
        ToggleStatusPanel,
        /// Frame-time overlay.
        ToggleFrameOverlay,
        /// Quit the app.
        Quit,
    ]
);

/// Registers key bindings (design.md §4 Keys tables). `secondary` is Cmd on Apple platforms
/// and Ctrl elsewhere (never both).
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewSession, None),
        KeyBinding::new("secondary-o", OpenFolder, None),
        KeyBinding::new("secondary-b", ToggleSidebar, None),
        KeyBinding::new("secondary-j", ToggleSidePane, None),
        KeyBinding::new("secondary-`", ToggleBottom, None),
        KeyBinding::new("secondary-.", Abort, None),
        KeyBinding::new("secondary-k", CommandPalette, None),
        KeyBinding::new("secondary-shift-p", CommandPalette, None),
        KeyBinding::new("secondary-m", PickModel, None),
        KeyBinding::new("secondary-shift-m", PickThinking, None),
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("secondary-\\", SplitPane, None),
        KeyBinding::new("secondary-shift-t", TreeView, None),
        KeyBinding::new("secondary-f", FindInConversation, None),
        KeyBinding::new("secondary-l", FocusComposer, None),
        KeyBinding::new("secondary-shift-s", ToggleStatusPanel, None),
        KeyBinding::new("ctrl-alt-shift-f", ToggleFrameOverlay, None),
        KeyBinding::new("secondary-q", Quit, None),
    ]);
}
