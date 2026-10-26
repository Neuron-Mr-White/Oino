//! GPUI views (feature `views`): the ZCode-inspired desktop shell (gpui-app §5).

pub mod dialog;
pub mod ous;
pub mod palette;
pub mod session_pane;
pub mod theme;
pub mod workspace;

use gpui_kit::{App, KeyBinding};

pub use workspace::Workspace;

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
        /// Abort the active run.
        Abort,
        /// Command palette.
        CommandPalette,
        /// Model picker.
        PickModel,
        /// Thinking level picker.
        PickThinking,
        /// Settings.
        OpenSettings,
        /// Toggle light/dark.
        ToggleTheme,
        /// Quit the app.
        Quit,
    ]
);

/// Registers key bindings (gpui-app §5.5). `secondary` is Cmd on macOS, Ctrl elsewhere.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewSession, None),
        KeyBinding::new("secondary-o", OpenFolder, None),
        KeyBinding::new("secondary-b", ToggleSidebar, None),
        KeyBinding::new("secondary-j", ToggleSidePane, None),
        KeyBinding::new("secondary-.", Abort, None),
        KeyBinding::new("secondary-k", CommandPalette, None),
        KeyBinding::new("secondary-l", PickModel, None),
        KeyBinding::new("secondary-t", PickThinking, None),
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("secondary-q", Quit, None),
    ]);
}
